// syntaxcue-audiotap — captures macOS system audio (everything the user hears,
// mixed to mono) via the CoreAudio Process Tap API (macOS 14.2+) and streams it
// as raw PCM to stdout.
//
// Adapted from the working reference in ~/dev/ipr/audiotap.swift (a personal
// prototype script), restructured into named steps and given explicit exit
// codes so the calling Rust process can tell "no permission yet" apart from
// "unsupported OS" and "something else went wrong" — see docs/PRODUCT-BRIEF.md
// §3 (Variant A, macOS permission note) and §8 acceptance criteria, which both
// require the app to explain a missing permission rather than show an
// unexplained empty stream.
//
// Chosen over ScreenCaptureKit (the brief's default suggestion) because this
// is what the proven reference implementation already uses, and it only needs
// the narrower "System Audio Recording Only" privacy entry rather than full
// Screen Recording — see README.md "Architecture decisions" for the writeup.
//
// Output contract:
//   stdout — raw interleaved PCM, Float32, 16000 Hz, mono. Nothing else ever
//            goes to stdout.
//   stderr — human-readable log lines, each also starting with a machine
//            -parseable "STATUS:" token the Rust side matches on:
//              STATUS:ready                 — capture has started
//              STATUS:unsupported_os        — macOS is older than 14.2
//              STATUS:permission_denied:... — tap/aggregate device creation failed
//              STATUS:error:...             — anything else unexpected
//   exit codes: 0 clean shutdown (SIGINT/SIGTERM), 1 generic error,
//               2 permission denied, 3 unsupported OS.
//
// `--probe`: added 2026-07-30 for the setup-screen "Permissions" section
// (docs/TODO.md). There is no public preflight API for this TCC entry —
// unlike Screen Recording, checking it and requesting it are the same
// action. This mode does exactly that and nothing else: create the tap
// (which is what actually triggers/consults the system permission), then
// immediately tear it down and exit — STATUS:probe_ok on success, the same
// STATUS:permission_denied as the real capture path on failure. No audio is
// ever read or written in this mode.

import Foundation
import CoreAudio
import AudioToolbox
import AVFoundation

let outSampleRate = 16000.0
let stdout = FileHandle.standardOutput
let stderr = FileHandle.standardError

func statusLine(_ s: String) {
    stderr.write((s + "\n").data(using: .utf8)!)
}

func fail(_ code: Int32, _ status: String) -> Never {
    statusLine(status)
    exit(code)
}

func checkMinimumOS() {
    let min = OperatingSystemVersion(majorVersion: 14, minorVersion: 2, patchVersion: 0)
    guard ProcessInfo.processInfo.isOperatingSystemAtLeast(min) else {
        fail(3, "STATUS:unsupported_os: requires macOS 14.2 or later")
    }
}

/// Creates a private, unmuted, global system-audio tap. Failure here is the
/// primary signal we treat as "permission not granted yet" — CoreAudio does
/// not expose a way to check the "System Audio Recording Only" TCC entry
/// ahead of time, so like Chrome/Zoom/OBS we just attempt the capture and
/// react to failure.
func createSystemTap() -> (AudioObjectID, UUID) {
    let description = CATapDescription(monoGlobalTapButExcludeProcesses: [])
    description.uuid = UUID()
    description.isPrivate = true
    description.muteBehavior = .unmuted // the user keeps hearing their own audio

    var tapID = AudioObjectID(kAudioObjectUnknown)
    let err = AudioHardwareCreateProcessTap(description, &tapID)
    guard err == noErr, tapID != kAudioObjectUnknown else {
        fail(2, "STATUS:permission_denied: could not create an audio tap (OSStatus \(err)) — System Audio Recording permission is likely missing")
    }
    return (tapID, description.uuid)
}

func readTapFormat(_ tapID: AudioObjectID) -> AudioStreamBasicDescription {
    var address = AudioObjectPropertyAddress(
        mSelector: kAudioTapPropertyFormat,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain)
    var asbd = AudioStreamBasicDescription()
    var size = UInt32(MemoryLayout<AudioStreamBasicDescription>.size)
    let err = AudioObjectGetPropertyData(tapID, &address, 0, nil, &size, &asbd)
    guard err == noErr else {
        fail(1, "STATUS:error: could not read tap format (OSStatus \(err))")
    }
    return asbd
}

/// Wraps the tap in a private aggregate device — CoreAudio only delivers
/// audio through a device's IO cycle, taps aren't readable on their own.
func createAggregateDevice(wrapping tapUUID: UUID) -> AudioObjectID {
    let description: [String: Any] = [
        kAudioAggregateDeviceNameKey: "syntaxcue-audiotap-aggregate",
        kAudioAggregateDeviceUIDKey: UUID().uuidString,
        kAudioAggregateDeviceIsPrivateKey: true,
        kAudioAggregateDeviceTapAutoStartKey: true,
        kAudioAggregateDeviceTapListKey: [
            [kAudioSubTapUIDKey: tapUUID.uuidString,
             kAudioSubTapDriftCompensationKey: true]
        ],
    ]
    var aggregateID = AudioObjectID(kAudioObjectUnknown)
    let err = AudioHardwareCreateAggregateDevice(description as CFDictionary, &aggregateID)
    guard err == noErr, aggregateID != kAudioObjectUnknown else {
        fail(2, "STATUS:permission_denied: could not create the aggregate capture device (OSStatus \(err))")
    }
    return aggregateID
}

/// Set once `startCapture` succeeds; read back by `cleanupAndExit` so a
/// signal doesn't leave the tap/aggregate device dangling — see the note by
/// the signal handlers below for why an orphaned tap matters.
var runningProcID: AudioDeviceIOProcID?

func startCapture(aggregateID: AudioObjectID, inFormat: AVAudioFormat) {
    guard let outFormat = AVAudioFormat(
        commonFormat: .pcmFormatFloat32, sampleRate: outSampleRate, channels: 1, interleaved: true
    ) else {
        fail(1, "STATUS:error: could not build output format")
    }
    guard let converter = AVAudioConverter(from: inFormat, to: outFormat) else {
        fail(1, "STATUS:error: could not build sample-rate converter")
    }

    var procID: AudioDeviceIOProcID?
    let err = AudioDeviceCreateIOProcIDWithBlock(&procID, aggregateID, nil) { _, inputData, _, _, _ in
        guard let inBuffer = AVAudioPCMBuffer(pcmFormat: inFormat, bufferListNoCopy: inputData, deallocator: nil),
              inBuffer.frameLength > 0 else { return }

        let capacity = AVAudioFrameCount(Double(inBuffer.frameLength) * outSampleRate / inFormat.sampleRate) + 16
        guard let outBuffer = AVAudioPCMBuffer(pcmFormat: outFormat, frameCapacity: capacity) else { return }

        var fed = false
        var conversionError: NSError?
        let status = converter.convert(to: outBuffer, error: &conversionError) { _, inputStatus in
            if fed {
                inputStatus.pointee = .noDataNow
                return nil
            }
            fed = true
            inputStatus.pointee = .haveData
            return inBuffer
        }
        guard status != .error, outBuffer.frameLength > 0, let channel = outBuffer.floatChannelData else { return }
        stdout.write(Data(bytes: channel[0], count: Int(outBuffer.frameLength) * 4))
    }
    guard err == noErr, procID != nil else {
        fail(1, "STATUS:error: could not create the audio IO callback (OSStatus \(err))")
    }

    let startErr = AudioDeviceStart(aggregateID, procID)
    guard startErr == noErr else {
        fail(1, "STATUS:error: could not start the capture device (OSStatus \(startErr))")
    }
    runningProcID = procID
}

checkMinimumOS()

let (tapID, tapUUID) = createSystemTap()

if CommandLine.arguments.contains("--probe") {
    AudioHardwareDestroyProcessTap(tapID)
    statusLine("STATUS:probe_ok")
    exit(0)
}

let asbd = readTapFormat(tapID)
statusLine("STATUS:info: tap format \(asbd.mSampleRate) Hz, \(asbd.mChannelsPerFrame) channel(s)")

var mutableAsbd = asbd
guard let inFormat = AVAudioFormat(streamDescription: &mutableAsbd) else {
    fail(1, "STATUS:error: tap reported an unusable audio format")
}

let aggregateID = createAggregateDevice(wrapping: tapUUID)
startCapture(aggregateID: aggregateID, inFormat: inFormat)

statusLine("STATUS:ready")

// The old handler just called exit(0), leaving the tap and aggregate device
// dangling — CoreAudio does eventually reclaim resources from a dead
// process, but not always promptly, and a stray tap/aggregate device from an
// unclean previous run has been observed to make a subsequent run's tap
// silently capture no audio (found 2026-07-29 debugging a "Start listening"
// session that produced only zero-byte PCM). Tearing these down explicitly
// on exit removes that failure mode.
//
// Raw `signal()` handlers must be async-signal-safe — calling into CoreAudio
// (heap allocation, XPC) from one risks a deadlock or crash. `DispatchSource`
// defers the actual handler to the given queue instead of running inline on
// the signal, so it's safe to do real work here.
func cleanupAndExit() -> Never {
    if let procID = runningProcID {
        AudioDeviceStop(aggregateID, procID)
        AudioDeviceDestroyIOProcID(aggregateID, procID)
    }
    AudioHardwareDestroyAggregateDevice(aggregateID)
    AudioHardwareDestroyProcessTap(tapID)
    exit(0)
}

signal(SIGINT, SIG_IGN)
signal(SIGTERM, SIG_IGN)
let sigintSource = DispatchSource.makeSignalSource(signal: SIGINT, queue: .main)
sigintSource.setEventHandler { cleanupAndExit() }
sigintSource.resume()
let sigtermSource = DispatchSource.makeSignalSource(signal: SIGTERM, queue: .main)
sigtermSource.setEventHandler { cleanupAndExit() }
sigtermSource.resume()

dispatchMain()

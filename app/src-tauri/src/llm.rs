// Answer generation through Claude Code (Agent SDK), via a long-lived Node
// sidecar (app/agent-sidecar/sidecar.mjs).
//
// The sidecar keeps ONE Agent SDK session open for the whole interview, so a
// question costs one round trip instead of a process spawn (~0.8s to the
// first word measured vs ~1.5s+ when the session is recreated per question).
// Rust talks to it with one JSON object per line over stdin/stdout, and turns
// its `delta` lines into the same `llm-delta` / `llm-done` / `llm-error`
// events the frontend already listens to.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::oneshot;

const DEFAULT_MODEL: &str = "claude-haiku-4-5";

/// Prefix the model is told to put on the off-topic refusal, and nothing
/// else. Exists so a refusal can be recognised without matching on its
/// wording: the reply is one free-form sentence in whatever the user set as
/// the interview language, so there is no phrase to match against. A refusal
/// delivers nothing, so it must not cost one of the free hints — a mis-aimed
/// screenshot burning a hint mid-interview was found in the 2026-08-06
/// clean-Mac run (docs/TODO.md, finding #5).
///
/// Stripped before the text reaches the panel (see `split_no_task_marker`).
/// If the model ignores the instruction the marker simply isn't there, the
/// answer streams unchanged and the hint is counted exactly as before — the
/// failure mode is today's behaviour, never a visible marker.
const NO_TASK_MARKER: &str = "NO_TASK_FOUND";

/// Marker the model is told to open with when `AnswerRequest::translate_question`
/// is set — ported from interview-app's llm.rs (owner decision 2026-10-01).
/// Stripped before the text reaches the panel; the question text after it is
/// emitted as its own `translated-question` event instead.
const TRANSLATION_MARKER: &str = "TRANSLATED_Q:";

/// Splits the buffered start of an answer into "was this the off-topic
/// refusal" and the text that should actually reach the panel.
fn split_no_task_marker(head: &str) -> (bool, String) {
    match head.trim_start().strip_prefix(NO_TASK_MARKER) {
        // Models tend to follow a marker like this with a separator of their
        // own choosing (": ", " — ", a newline) before the sentence itself.
        Some(rest) => (
            true,
            rest.trim_start_matches(|c: char| c.is_whitespace() || c == ':' || c == '-' || c == '—')
                .to_string(),
        ),
        None => (false, head.to_string()),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnswerRequest {
    pub model: String,
    /// Free-text role/company the user is interviewing for.
    pub interview_context: String,
    /// Language the answer (prose and code comments alike) should be written
    /// in — the same language the user picked for speech recognition; see
    /// brief §5 "Язык" 2026-08-02 addendum for why this is one setting, not
    /// two.
    pub response_language: String,
    /// The question or problem, as text. Empty when only an image is sent.
    pub text: String,
    /// A pasted/screenshotted problem as a PNG data URI, if any.
    pub image_data_uri: Option<String>,
    /// Company card name (profiles/companies/<name>.md), empty = none.
    #[serde(default)]
    pub company: Option<String>,
    /// Knowledge profile (profiles/<name>/bot/knowledge.md), empty = first one.
    #[serde(default)]
    pub profile: Option<String>,
    /// True only for a question that came from live speech recognition
    /// (frontend sets it for `kind: "heard"`) — triggers the Russian
    /// translation of the question (see `TRANSLATION_MARKER`) so the panel
    /// reads in Russian even for a non-Russian interview. Ported from
    /// interview-app's llm.rs (owner decision 2026-10-01).
    #[serde(default)]
    pub translate_question: bool,
}

#[derive(Debug, Serialize, Clone)]
struct StreamError {
    message: String,
}

/// The one answer-format instruction (the old "Answer style" setting is gone, owner
/// decision 2026-10-24: the later length rules for conceptual / behavioral / system-design
/// questions overrode it anyway). It hands the format choice to the model: a plain factual
/// question and a multi-part system-design question don't belong in the same shape of answer.
const ANSWER_FORMAT: &str = "Match the format to the question. For a direct/factual question, \
     answer in 1-2 sentences. For a conceptual or system-design question \
     involving trade-offs or multiple components, give the main point \
     in the first line, then at most 3 more short sentences of 8-15 \
     words each, every one on its own line, no bullet markers. Pick \
     only the most important points; the interviewer will ask for \
     details if they want them. Use plain everyday words, the way a \
     person talks, not the way documentation reads. No preamble, no \
     closing summary.";

fn build_system_prompt(req: &AnswerRequest) -> String {
    let mut prompt = String::new();

    // Genericized from "a software engineer during a live technical interview"
    // 2026-08-27 (docs/TODO.md, "Системный промпт жёстко привязан к 'software
    // engineer' + 'technical interview'") — that framing, combined with the
    // off-topic guard below, was refusing legitimate behavioral/general
    // questions ("tell me about a time you handled conflict on a team") even
    // for the existing developer audience, who get those mixed into a
    // technical loop too, not just a hypothetical general-audience user.
    // Extended 2026-10-01 to match interview-app's llm.rs (owner decision: port
    // its answer-voice rules here) — "glance" framing, the B1-B2 non-native
    // style, and the no-moral-ending rule. The B1-B2 sentence is conditioned on
    // the answer language: unlike interview-app (English-only product), this
    // app also answers in Russian (CLAUDE.md "Язык"), and Russian is the
    // candidate's native language — "not like a native speaker" would be false
    // and could push the model to simplify Russian answers it shouldn't.
    prompt.push_str(
        "You help a candidate during a live interview. They are speaking to an \
         interviewer right now and will read or paraphrase your answer out loud \
         within seconds, so brevity and directness matter more than \
         completeness. The candidate must be able to glance at the answer for \
         1-2 seconds and get the main point before speaking — optimize for \
         being spoken from a screen, not for being read silently. If you must \
         choose between fitting in one more true detail and keeping the answer \
         easy to say out loud, drop the detail. ",
    );
    if req.response_language.trim() != "Russian" {
        prompt.push_str(
            "The candidate speaks English at a solid intermediate level (B1-B2), \
             not like a native speaker or a professional writer: use simple, \
             common, everyday words and short sentences everywhere, even in \
             technical answers, and do not reach for a more sophisticated \
             synonym when a plain one works just as well. ",
        );
    }
    prompt.push_str(
        "This rule applies to every answer, technical or not: never end with a \
         summarizing moral or takeaway (\"The lesson was...\", \"This shows \
         that...\", \"Overall...\", \"which means I can...\", \"so I know how \
         to...\") — stop as soon as the point is made (the last concrete fact \
         or result) and let the interviewer draw their own conclusion.\n\n",
    );

    // Ported from interview-app's llm.rs (owner decision 2026-10-01): when the
    // question came from live speech recognition (`translate_question`, set by
    // the frontend only for `kind: "heard"`), the model opens with a Russian
    // translation of the question on its own first line so the panel reads in
    // Russian even though the interview itself (recognition + answer) stays in
    // the interview's own language end to end (CLAUDE.md "Язык"). Display-only
    // — never changes what language the actual answer is given in.
    if req.translate_question {
        prompt.push_str(&format!(
            "Before anything else, on its own first line, output \"{TRANSLATION_MARKER} \" \
             followed by a natural Russian translation of the candidate's question below \
             (not the answer — just the question, translated). Then a newline, then continue \
             exactly as instructed for the rest of this reply. Nothing else goes on that \
             first line.\n\n"
        ));
    }

    // Reversed 2026-08-16 (owner decision, overriding the 2026-08-01 synthesis
    // recorded in docs/PRODUCT-BRIEF.md addendum "Язык ответа vs Preferred
    // language/framework" — see the "Разворот приоритета 2026-08-16" note
    // appended to that same addendum) — a set preference is now a *hard*
    // choice, not a fallback the
    // screen can override. Reasoning: a candidate who set this field and then
    // lands on a problem in a different language typed it in themselves; a
    // stale/forgotten setting is on them to fix, not something the app should
    // silently second-guess by reading the screen instead. Simpler and more
    // predictable than the screen-priority rule it replaces, at the cost of
    // occasionally answering in a language the problem's own scaffolding
    // won't actually accept.
    let has_context = !req.interview_context.trim().is_empty();
    if has_context {
        prompt.push_str(
            "Role, company, and tech stack they are interviewing for (may include \
             a programming language or framework — see the language selection \
             order below for exactly how much weight that carries): ",
        );
        prompt.push_str(req.interview_context.trim());
        prompt.push_str("\n\n");
    }

    prompt.push_str(ANSWER_FORMAT);
    prompt.push_str("\n\n");

    prompt.push_str(
        "System-design and architecture questions (\"design a URL shortener\", \"how would \
         you design ...\", \"how would you scale ...\") are NOT coding problems: write no \
         code for them. Give the direct approach in one line, then 4-6 short bullet points \
         covering the main components, the data model, how it scales, and the key \
         trade-offs.\n\n",
    );
    prompt.push_str("If the input is a coding problem, respond with a working solution as a \
         fenced code block. Choose the language in this order: ");
    if has_context {
        prompt.push_str(
            "(1) if a language/framework was given above, use it — always, even if \
             the problem's own on-screen scaffolding (boilerplate, function \
             signature, judge/IDE environment) shows a different language. Do not \
             let the screen override this. Never refuse, hedge, or give a reduced \
             answer just because the problem's visible language differs from this \
             one — answer fully in the language given above regardless. A \
             one-clause note about the mismatch is fine; withholding the answer is \
             not. (2) If nothing was given above, fall back to whatever language \
             the problem's own scaffolding indicates. (3) Failing that, default \
             to Python. ",
        );
    } else {
        prompt.push_str(
            "(1) match whatever language the problem's own on-screen scaffolding \
             (boilerplate, function signature, judge/IDE environment) or an \
             explicit verbal request indicates; (2) otherwise default to Python. ",
        );
    }
    // Found 2026-08-16 live-testing the screenshot flow: language alone isn't
    // enough — a LeetCode-style judge's own starter code is frequently visible
    // (a signature, a class wrapper or its absence) and the model was seen
    // ignoring it, defaulting to whatever wrapper it reaches for out of habit
    // for that language (e.g. wrapping a JS answer in a class, which no
    // LeetCode JS template does — JS ships a plain function while
    // C++/Java/Python's templates do use a class). Only meaningful when the
    // answer's language actually matches what's on screen — if the preference
    // above overrode the screen's language, there is no on-screen shape to
    // reproduce in a *different* language, so this falls back to each
    // language's own idiomatic default instead (plain function for JS, no
    // invented class).
    prompt.push_str(
        "If the problem's own scaffolding is visible on screen AND you're \
         answering in that same language, reproduce its exact shape — the \
         starter function signature, a class wrapper (or its absence), \
         parameter names — rather than whatever wrapper you'd otherwise \
         default to for that language; the visible scaffolding is more specific \
         evidence of what the judge expects than a general per-language \
         convention is. If you're answering in a different language than what's \
         shown on screen (per the preference above overriding it), there is \
         nothing to reproduce — just use that language's own normal idiomatic \
         shape (e.g. a plain function for JavaScript, not an invented class).\n\n",
    );
    // Added 2026-09-15 per docs/PROPOSAL-ANSWER-FORMAT-MY-THOUGHTS.md (owner
    // decision, Cluely-inspired): a short spoken-approach layer before the
    // code, matching how a candidate actually talks through a problem —
    // state the approach out loud first, then type. This is additive, not a
    // replacement for the in-code comments below: the "My Thoughts" block is
    // for the moment before typing starts, the comments are for the moment of
    // typing itself. Explicitly not a restated problem statement (rejected by
    // the owner) — just the candidate's own reasoning/approach.
    prompt.push_str(
        "Before the code, add 2-4 short bullet points, each on its own line \
         starting with \"- \", giving your approach/reasoning — which \
         technique or data structure you'll use and why, not a restatement of \
         the problem. Plain prose after the dash, no markdown headers or bold, \
         and no title or label line before the first bullet — the UI adds its \
         own heading, so start directly with the first \"- \" line. Keep it \
         brief — this is said out loud before typing starts, not a written \
         essay.\n\n",
    );
    // The explanation lives in the code as comments rather than in a section
    // below it: the candidate types the code top to bottom while narrating,
    // and a separate block means reading one place while typing another. Each
    // comment sits on the step it describes, so the thing to say next is
    // always at the line being typed.
    //
    // The tradeoff accepted here is that comments travel with the code into
    // the interview's own editor when the candidate types or pastes them.
    // That's why they must read as ordinary engineering comments — what the
    // step does and why — and never as stage directions to the candidate.
    prompt.push_str(
        "Put the explanation in the code itself, as comments. Comment each \
         meaningful step with what it does and why that choice, phrased so the \
         candidate can say it out loud while typing that line. Keep each comment \
         a short fragment, not a full sentence, and skip trivial lines. Write \
         them as normal engineering comments about the code — never as \
         instructions to the candidate, and never mention the interview, the \
         interviewer, or saying anything out loud. Place every comment as a \
         trailing comment on the same line as the code it describes (e.g. \
         `total += price  # running sum`), never on its own line above the \
         code — if a line is already long, shorten the comment to fit rather \
         than moving it above. Use a plain two-space gap before the `#`, the \
         same on every line — do NOT pad shorter lines with extra spaces to \
         make every `#` line up in a column; that column-alignment style \
         pushes the comment far from short lines and is not what's wanted \
         here. Only when your answer contains code, add after it exactly two \
         lines giving complexity, in this exact format and nothing else on \
         those lines: `Time: O(...)` then `Space: O(...)`. Never write \
         complexity lines in an answer without code (behavioral, verbal or \
         conceptual answers). Do not add a separate explanation section \
         after the code — the comments are the explanation.\n\n",
    );
    // Added 2026-10-08 (owner, live-coding test): when the task on screen
    // lists follow-up questions, the model wrote them as long "Follow-up N:"
    // paragraphs below the code, so answering an interviewer's follow-up
    // meant scrolling away from the code it's about. They now live in the
    // code as short tagged comments on the line they concern.
    prompt.push_str(
        "If the problem on screen lists follow-up questions, answer each one \
         as a short trailing comment tagged `FU1:`, `FU2:`, … (same comment \
         syntax as the other comments) on the code line it concerns — e.g. \
         the data-structure line for a scaling or memory question, the \
         eviction/loop line for an ordering or edge-case question. Keep each \
         to one short line. If a follow-up has no matching line, put it as a \
         short `FUn:` comment on the closest relevant line anyway. Never add \
         a separate \"Follow-up\" section after the code.\n\n",
    );

    // Explicit positive branch for the non-coding case, parallel to the
    // coding-problem block above. Without this, the heavy code-specific
    // instructions above are the only concrete guidance in the prompt, and
    // the model can default to treating anything that isn't a coding problem
    // as out of scope. Widened 2026-08-27 (docs/TODO.md) beyond "technical"
    // to genuinely any *factual/conceptual* subject an interview might
    // cover — owner's own framing: the LLM doesn't care what domain a
    // question is in, there's no reason for this prompt to artificially
    // restrict it. This is a capability change only, not a marketing one —
    // the site's public positioning stays scoped to technical/engineering
    // interviews (docs/PRODUCT-BRIEF.md §2, unchanged); same pattern already
    // used for multilingual support (CLAUDE.md "Побочный эффект" — a quiet
    // capability in the product, not an advertised feature).
    //
    // Deliberately does NOT include behavioral/personal-experience questions
    // — those get their own, differently-shaped branch right below. The
    // distinction isn't subject matter, it's whether a correct answer exists
    // independent of who's answering (technical, accounting, legal — yes) or
    // requires the candidate's own real, specific history (behavioral — no).
    //
    // 2026-10-12 (owner: answers shorter, simpler, more natural): the former
    // `Answer: <headline>` + "fuller explanation" paragraph is replaced by a
    // direct length rule. Measured on 14 conceptual questions, Sonnet 5, 2-3
    // repeats: median 86 -> 63 words, max 143 -> 138.
    prompt.push_str(
        "If the input is a conceptual, system-design, verbal technical, or other \
         factual/conceptual interview question — technical or not (e.g. an \
         accounting or legal question) — rather than one asking you to write code \
         or describe a personal experience, answer it from your own knowledge of \
         that subject like this: the main point in one sentence, then at most 3 \
         more short sentences of 8-15 words each, every one on its own line. No \
         bullet lists, no headline, no code unless asked, plain everyday words. \
         The interviewer will ask for details if they want them. Do not treat it \
         as off-topic.\n\n",
    );

    // Behavioral / motivational / work-condition questions are answered ONLY
    // from the candidate's own knowledge base (appended at the very end, see
    // knowledge.rs). The owner supplies real facts; the model turns them into
    // ready-to-read spoken answers and never invents history. The model decides
    // per question whether it is behavioral or technical: measured 2026-09-19,
    // a forced "behavioral" tag made technical answers open with meta talk.
    prompt.push_str(
        "If the question is behavioral, about personal experience, motivation or company \
         fit, or about work conditions (\"tell me about a time...\", \"why do you want to \
         work here?\", \"what are your salary expectations?\", \"why are you leaving?\"), \
         answer ONLY from the CANDIDATE KNOWLEDGE BASE at the end of this prompt. \
         Find the entry that matches the question and give its prepared answer as \
         ready-to-read text in the answer language (translate simply if the entry is in \
         the other language), adapted to the exact wording of the question. Keep the \
         candidate's simple spoken style: short sentences, contractions, plain words, \
         as spoken by a non-native speaker with intermediate English. Put every \
         short sentence on its own line, with no blank lines inside one answer, so it \
         can be read at a glance. Do not make the answer longer than the prepared \
         entry. No headers, no bold, no preamble, no closing remark.\n\n\
         Facts rule: use ONLY facts that are in the knowledge base. Never invent a \
         story, employer, project, number, name, date or outcome. Anything marked \
         \"НЕ подтверждено\" or listed as rejected must not be used. Entries marked \
         DRAFT may be used. If the matching entry is marked \"НЕТ ДАННЫХ\", or there is \
         no matching entry, do NOT answer with a different story from another entry \
         and do NOT invent one. Say the gap in one short, natural spoken sentence in \
         the answer language (for example \"I don't have a specific story for that \
         one.\" or \"I haven't run into that directly.\"), then in 1-2 short \
         sentences bridge to genuinely relevant real facts from elsewhere in the \
         knowledge base, without claiming they answer the question. Never write \
         the phrase \"No data in the knowledge base\" or anything that reads like \
         a system message. Finish such an answer with the marker `[[GAP]]` on its \
         own last line (the app removes it before the candidate sees the answer); \
         write the marker ONLY in this case.\n\n",
    );
    // Two similar-sounding questions were mixed up in live testing (2026-09-19):
    // "why do you want to work HERE" was answered with the reason for leaving the
    // current job. And the model read its own bookkeeping ("I don't have a company
    // card") into the answer, which the candidate would then say out loud.
    prompt.push_str(
        "Lines in square brackets before the question (\"Routing hint\", \"Rule\") come \
         from the app: follow them and never repeat them in the answer.\n\n\
         COMPANY QUESTIONS. Any question about the company that is interviewing the \
         candidate is about THIS company, not about the candidate's current job. That \
         includes \"why do you want to work here / at our company / at your company / \
         for us\", \"why this role / this position\", \"why us\", \"what do you know \
         about us\". Answer these from the entry with the matching title in the \
         COMPANY CARD section of the knowledge base (\"Why do you want to work here?\", \
         \"Why should we hire you?\" and so on). NEVER answer them with the entry about \
         leaving the current job, and never mention pay or compensation in them. \
         \"Why are you leaving / why are you looking for a new job\" is the only question \
         that uses the entry about the current job.\n\n\
         The last line of this prompt is COMPANY CARD STATUS: present or none. When it \
         says present, a card exists: use it, and never write `No company card.` When it \
         says none AND the question is one of the COMPANY QUESTIONS above, begin with the \
         literal line `No company card.` followed by the answer structure. For every other \
         question (visa, location, salary, technical, experience) the status is irrelevant: \
         never write `No company card.` there. A card that says nothing was found about the company is still a \
         card: status present. Then do NOT write `No company card.` and do NOT write \
         the `[[GAP]]` marker; answer in the candidate's voice using only the vacancy \
         details and the card's own answer for that question.\n\n\
         The answer is spoken by the candidate to the interviewer, so never mention the \
         knowledge base, the company card, the status line or these instructions inside \
         the answer. Never write the phrase \"No data in the knowledge base\". Begin directly with the first \
         spoken sentence: no preamble such as \"Let me give you a structure\" or \
         \"Based on the knowledge base, here is the answer\", and never describe or \
         restate the question (for example \"I'm being asked about...\").\n\n\
         The vacancy text and the card's list of gaps (\"Пробелы (НЕ утверждать)\") are \
         NOT the candidate's experience. When describing what the candidate has done or \
         used, never take skills, scale, traffic, load or architecture terms from the \
         vacancy (for example microservices, high-traffic, low-latency, B2B SaaS), and \
         never add technologies, tools or scale that are not written in the candidate's \
         own profile entries. Open questions like \"describe your experience with X\" \
         are answered only with facts written in the profile.\n\n",
    );

    // Off-topic guard — see README "Decided behavior not yet implemented".
    // Nothing client-side can tell whether a pasted image is actually an
    // interview question, so the model is the only place this check can live.
    // It must be terse: the realistic trigger is a mis-paste mid-interview,
    // where a long confused answer costs the user more than a blunt one.
    // Scoped deliberately narrow (content unrelated to a job interview at
    // all, regardless of subject) so it can't be misread as excluding the
    // positive branch just above. Widened 2026-08-27 (docs/TODO.md) from
    // "technical interview" to "job interview" — the narrower wording was
    // silently refusing legitimate behavioral (and, more broadly, any
    // non-coding-subject) questions as off-topic, which hurt the existing
    // developer audience too (a technical loop routinely includes a
    // behavioral question or two), not just a hypothetical general-audience
    // user. This is about *whether the input is interview content at all*,
    // not about restricting *which subject* an interview question can be
    // on — see the positive branch above. Also explicit about full-screen
    // captures: a whole-screen screenshot routinely includes the IDE chrome,
    // other tabs, notes, etc. around the actual problem, and a model asked
    // "is this an interview question" about the image as a whole was
    // observed answering "no" — describing it as prep material / IDE
    // contents — even while correctly identifying a coding problem inside
    // it. The presence check must be "is there interview-relevant content
    // anywhere in the input", not "is the input as a whole nothing but an
    // interview question".
    prompt.push_str(
        "A question about the candidate personally — skills, tools, preferences, \
         opinions, habits, experience (\"what is your favorite programming language?\", \
         \"what tools do you use?\") — is ALWAYS an interview question, never off-topic. \
         Answer it ONLY with facts that are in the knowledge base (for example the \
         technology stack listed there). NEVER invent a preference, opinion, tool, \
         editor, habit or reason that the knowledge base does not state: the technology \
         stack is a fact, but \"my favorite is X\" or \"I use VS Code\" is not unless \
         written there. Every sentence you write about the candidate must be traceable to \
         a fact or a prepared answer in the knowledge base: do not add reasons, \
         qualifications, comparisons or opinions of your own (for example \"but not \
         completely alone\" or \"without the verbosity of Java\"). When the base has \
         nothing on the exact point, follow the no-data rule above, and you may add the \
         relevant facts that do exist.\n\n",
    );
    prompt.push_str(
        "If the input has nothing to do with a job interview at all (e.g. an \
         unrelated screenshot or an accidental paste), reply with exactly one short \
         sentence saying so and nothing else. Do not analyze or describe it. This \
         does not apply to any interview question covered by the two paragraphs \
         above (conceptual, system-design, verbal technical, any other \
         factual/conceptual subject, or behavioral) — those always get a real \
         answer (from the knowledge base for behavioral, per that paragraph), never this \
         one-sentence reply. A full-screen screenshot may also show an IDE, other \
         windows, or unrelated content around the actual problem — if a coding \
         problem or interview-relevant question is visible ANYWHERE in the image, \
         that counts as on-topic and must get a real answer; judge relevance by \
         whether such content is present at all, not by whether it fills the whole \
         image or by what else surrounds it.\n\n",
    );

    // Screenshot + recent speech (owner request 2026-11-02): the frontend adds
    // what the interviewer said just before the screenshot, so the request
    // asked in words is not guessed from the picture alone.
    prompt.push_str(
        "If the input contains a line starting with \"[Interviewer said just before \
         this screenshot:\", that is the interviewer's latest spoken request and the \
         attached image is its context. Answer THAT request, using the image only as \
         the material it refers to (code, diagram, error, document, task). The spoken \
         request may not be about coding at all (for example a question about the \
         architecture, a behavioral question, or a request to explain something on \
         screen); never default to solving a coding problem just because one is \
         visible. The speech is transcribed and may contain earlier unrelated lines: \
         use the part that is the request. If the speech is clearly unrelated to the \
         image, answer the speech. If it is only small talk, fall back to the image \
         alone.\n\n",
    );

    // Machine-readable tag on that one refusal path, so the app can tell a
    // refusal from an answer without matching on its wording — see
    // NO_TASK_MARKER. Stated as its own paragraph, after the guard above has
    // finished scoping *when* the refusal applies, so this only ever adds a
    // prefix to that case and never widens it.
    prompt.push_str(&format!(
        "When — and only when — that one-sentence reply applies, begin it with the \
         literal token {NO_TASK_MARKER}, in capitals and in English no matter which \
         language you are answering in, followed by the sentence itself. Never write \
         that token in any other reply.\n\n"
    ));

    // Required because this runs with thinking off for latency (see the
    // `thinking` field in anthropic_body): without it, the model may write its
    // reasoning into the visible response instead. Phrase this as what to
    // produce, never as "do not think" — an instruction against reasoning
    // makes leaked `<thinking>` tags more likely, not less.
    //
    // The code-specific sentence was added after a live test (docs/TODO.md,
    // "Модель иногда пишет невалидный/черновой код...") showed the general
    // instruction above wasn't enough on its own: the model wrote an invalid
    // line, then visibly corrected itself mid-stream ("Let me correct that
    // stray line: ..."), which is exactly the failure mode this whole
    // paragraph exists to prevent. Naming the concrete pattern (wrong line +
    // visible correction) rather than repeating the abstract rule again.
    prompt.push_str(
        "Respond only with your final answer. No exploratory reasoning, no \
         restating the question, no meta-commentary about your process. If your \
         answer includes code, write only the single correct, complete version — \
         never write an incorrect or placeholder line and then visibly correct it \
         (e.g. \"let me fix that\" or a second corrected version below the \
         first).\n\n",
    );

    prompt.push_str("Respond in ");
    let language = if req.response_language.trim().is_empty() {
        "English"
    } else {
        req.response_language.trim()
    };
    prompt.push_str(language);
    prompt.push_str(". This includes code comments if the answer is a coding solution — write them in ");
    prompt.push_str(language);
    prompt.push_str(" too, not English, unless the answer language already is English.");

    let kb = crate::knowledge::load(req.profile.as_deref().unwrap_or(""), req.company.as_deref().unwrap_or(""));
    if kb.is_empty() {
        prompt.push_str("\n\nThere is no candidate knowledge base: for behavioral questions say so.");
    } else {
        prompt.push_str("\n\n# CANDIDATE KNOWLEDGE BASE\n");
        prompt.push_str(&kb);
    }
    prompt.push_str(if kb.contains("=== COMPANY CARD:") {
        "\n\nCOMPANY CARD STATUS: present"
    } else {
        "\n\nCOMPANY CARD STATUS: none"
    });

    prompt
}

// A fast answer (a short refusal, a one-line factual answer, the
// free-tier-exhausted message) can finish rendering inside a single
// throttled compositor interval for a non-key/background-app window — this
// app deliberately never takes focus on an arriving hint (CLAUDE.md), so the
// compact hint panel is almost always non-key during a real call, and macOS
// can leave its DOM changes sitting in WebKit's internal layer without ever
// flushing them to the actual screen until something else (a click) forces
// it. See docs/TODO-DONE.md "repaint-on-inactive-window" for the full
// investigation, including two failed geometry-nudge attempts before this
// one. This is a `#[tauri::command]` rather than something called
// automatically from `generate_answer`: the frontend calls it itself from
// inside a `requestAnimationFrame` callback right after writing the DOM
// (`main.ts`'s `llm-delta`/`llm-error`/`llm-done` listeners, and the
// free-tier-exhausted catch in `requestAnswer`), so the nudge fires only
// once WebKit itself believes it has painted, not immediately on stream
// completion — firing it from the Rust side immediately reproduced live as
// a race against WebKit's own internal commit. `objc2` is already in the
// dependency tree transitively via tauri/wry; added directly here just for
// this one message send. Never touches focus/key status — per CLAUDE.md's
// rule that focus is only ever taken on a user-initiated action,
// `displayIfNeeded` cannot violate that since it's a pure paint operation,
// not a window-ordering one.
#[tauri::command]
#[cfg(target_os = "macos")]
pub fn nudge_repaint(app: AppHandle) {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;

    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    let Ok(ptr) = win.ns_window() else {
        return;
    };
    if ptr.is_null() {
        return;
    }
    unsafe {
        let ns_window = ptr as *mut AnyObject;
        let _: () = msg_send![ns_window, displayIfNeeded];
    }
}

#[tauri::command]
#[cfg(not(target_os = "macos"))]
pub fn nudge_repaint(_app: AppHandle) {}


/// Splits a `data:image/png;base64,XXXX` URI into (media_type, base64 payload).
fn split_data_uri(uri: &str) -> Result<(String, String), String> {
    let rest = uri
        .strip_prefix("data:")
        .ok_or_else(|| "image is not a data URI".to_string())?;
    let (meta, data) = rest
        .split_once(",")
        .ok_or_else(|| "malformed data URI".to_string())?;
    let media_type = meta.trim_end_matches(";base64").to_string();
    Ok((media_type, data.to_string()))
}

/// The panel only knows Claude models; anything else a stale setting might
/// hold (an old OpenAI model name, an empty string) falls back to the fastest.
fn sanitize_model(model: &str) -> String {
    let m = model.trim();
    if m.starts_with("claude-") || matches!(m, "haiku" | "sonnet" | "opus") {
        m.to_string()
    } else {
        DEFAULT_MODEL.to_string()
    }
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<(), String>>>>>;
/// `AnswerRequest::translate_question` per question, pushed in `generate_answer`
/// (same order `ask` lines are written, which is the order their deltas arrive
/// in — the sidecar/Rust pipeline streams one answer at a time) and popped in
/// `read_events` when a new answer's first delta arrives. Ported from
/// interview-app (owner decision 2026-10-01).
type TranslateFlags = Arc<Mutex<std::collections::VecDeque<bool>>>;

struct Agent {
    /// Set when this agent is replaced, so its reader thread doesn't fail the requests of the next agent.
    retired: Arc<std::sync::atomic::AtomicBool>,
    child: Child,
    stdin: ChildStdin,
    system: String,
    model: String,
}

impl Drop for Agent {
    fn drop(&mut self) {
        self.retired.store(true, Ordering::SeqCst);
        let _ = self.child.kill();
    }
}

#[derive(Default)]
pub struct AgentState {
    agent: Mutex<Option<Agent>>,
    pending: Pending,
    next_id: AtomicU64,
    translate_flags: TranslateFlags,
}

fn write_line(stdin: &mut ChildStdin, value: &serde_json::Value) -> Result<(), String> {
    let mut line = value.to_string();
    line.push('\n');
    stdin
        .write_all(line.as_bytes())
        .and_then(|_| stdin.flush())
        .map_err(|e| format!("agent process is not accepting input: {e}"))
}

/// Turns the sidecar's stdout into frontend events. Runs until the sidecar
/// exits; every waiting `generate_answer` is failed at that point.
fn read_events(
    app: AppHandle,
    stdout: std::process::ChildStdout,
    pending: Pending,
    translate_flags: TranslateFlags,
    retired: Arc<std::sync::atomic::AtomicBool>,
) {
    // The first few characters of an answer are held back just long enough to
    // tell whether it opens with NO_TASK_MARKER (see `split_no_task_marker`);
    // after that every delta goes straight out.
    let mut head = String::new();
    let mut decided = false;

    // Translation-line state (ported from interview-app's llm.rs, owner
    // decision 2026-10-01): held back exactly like `head` above, but for the
    // `TRANSLATED_Q:` line a `translate_question` request opens with.
    // `current_translate` is `None` between answers and gets filled from
    // `translate_flags` the moment the first delta of a new answer arrives —
    // matching the `ask` send order, since only one answer streams at a time.
    let mut current_translate: Option<bool> = None;
    let mut translation_buffer = String::new();
    let mut translated = false;

    let flush_head = |app: &AppHandle, head: &mut String, decided: &mut bool| {
        if !*decided {
            let (_, visible) = split_no_task_marker(head);
            if !visible.is_empty() {
                let _ = app.emit("llm-delta", visible);
            }
        }
        head.clear();
        *decided = false;
    };

    // Emits the buffered translation line (if any) as a `translated-question`
    // event and resets the translation state for the next answer. Called both
    // when a newline settles the line normally and as a flush on `done`/`error`
    // (a stream that ends before the line ever saw a newline, e.g. a very
    // short/dropped response, would otherwise never emit it at all).
    let flush_translation = |app: &AppHandle,
                              translation_buffer: &mut String,
                              translated: &mut bool,
                              current_translate: &mut Option<bool>| {
        if *current_translate == Some(true) && !*translated && !translation_buffer.trim().is_empty() {
            let question = translation_buffer
                .trim()
                .strip_prefix(TRANSLATION_MARKER)
                .unwrap_or(translation_buffer.trim())
                .trim();
            if !question.is_empty() {
                let _ = app.emit("translated-question", question.to_string());
            }
        }
        translation_buffer.clear();
        *translated = false;
        *current_translate = None;
    };

    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let id = msg["id"].as_u64();
        match msg["type"].as_str() {
            Some("ready") => {
                let _ = app.emit("agent-ready", ());
            }
            Some("delta") => {
                let Some(mut text) = msg["text"].as_str() else { continue };
                if current_translate.is_none() {
                    current_translate = Some(translate_flags.lock().unwrap().pop_front().unwrap_or(false));
                }
                if current_translate == Some(true) && !translated {
                    translation_buffer.push_str(text);
                    let Some(line_end) = translation_buffer.find('\n') else {
                        continue;
                    };
                    let rest: String = translation_buffer.drain(line_end + 1..).collect();
                    let line = translation_buffer.trim();
                    let question = line.strip_prefix(TRANSLATION_MARKER).unwrap_or(line).trim();
                    if !question.is_empty() {
                        let _ = app.emit("translated-question", question.to_string());
                    }
                    translated = true;
                    if rest.is_empty() {
                        continue;
                    }
                    text = &rest;
                    if decided {
                        let _ = app.emit("llm-delta", text.to_string());
                        continue;
                    }
                    head.push_str(text);
                    if head.trim_start().chars().count() < NO_TASK_MARKER.chars().count() {
                        continue;
                    }
                    decided = true;
                    let (_, visible) = split_no_task_marker(&head);
                    if !visible.is_empty() {
                        let _ = app.emit("llm-delta", visible);
                    }
                    continue;
                }
                if decided {
                    let _ = app.emit("llm-delta", text.to_string());
                    continue;
                }
                head.push_str(text);
                if head.trim_start().chars().count() < NO_TASK_MARKER.chars().count() {
                    continue;
                }
                decided = true;
                let (_, visible) = split_no_task_marker(&head);
                if !visible.is_empty() {
                    let _ = app.emit("llm-delta", visible);
                }
            }
            Some("done") => {
                flush_head(&app, &mut head, &mut decided);
                flush_translation(&app, &mut translation_buffer, &mut translated, &mut current_translate);
                let _ = app.emit("llm-done", ());
                if let Some(tx) = id.and_then(|i| pending.lock().unwrap().remove(&i)) {
                    let _ = tx.send(Ok(()));
                }
            }
            Some("error") => {
                let message = msg["message"].as_str().unwrap_or("agent error").to_string();
                head.clear();
                decided = false;
                translation_buffer.clear();
                translated = false;
                current_translate = None;
                let _ = app.emit("llm-error", StreamError { message: message.clone() });
                if let Some(tx) = id.and_then(|i| pending.lock().unwrap().remove(&i)) {
                    let _ = tx.send(Err(message));
                }
            }
            _ => {}
        }
    }

    if retired.load(Ordering::SeqCst) {
        return;
    }
    let waiting: Vec<_> = pending.lock().unwrap().drain().collect();
    if !waiting.is_empty() {
        let message = "The answer engine stopped unexpectedly.".to_string();
        let _ = app.emit("llm-error", StreamError { message: message.clone() });
        for (_, tx) in waiting {
            let _ = tx.send(Err(message.clone()));
        }
    }
}

/// An app started from Finder does not inherit the terminal's PATH, so a plain
/// `node` is not found. Look in the usual install locations (newest nvm version
/// first); `NOTEPAD_NODE` overrides everything.
pub(crate) fn find_node() -> String {
    if let Ok(p) = std::env::var("NOTEPAD_NODE") {
        if !p.trim().is_empty() {
            return p;
        }
    }
    let home = std::env::var("HOME").unwrap_or_default();
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(format!("{home}/.nvm/versions/node")) {
        let mut versions: Vec<std::path::PathBuf> = entries.flatten().map(|e| e.path()).collect();
        versions.sort_by_key(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("").trim_start_matches('v').to_string();
            name.split('.').map(|n| n.parse::<u32>().unwrap_or(0)).collect::<Vec<_>>()
        });
        versions.reverse();
        candidates.extend(versions.into_iter().map(|v| v.join("bin/node")));
    }
    for fixed in [
        format!("{home}/.volta/bin/node"),
        "/opt/homebrew/bin/node".to_string(),
        "/usr/local/bin/node".to_string(),
        "/usr/bin/node".to_string(),
    ] {
        candidates.push(fixed.into());
    }
    candidates
        .into_iter()
        .find(|c| c.is_file())
        .map(|c| c.to_string_lossy().into_owned())
        .unwrap_or_else(|| "node".to_string())
}

fn spawn_agent(
    app: &AppHandle,
    pending: &Pending,
    translate_flags: &TranslateFlags,
    system: &str,
    model: &str,
) -> Result<Agent, String> {
    let node = find_node();
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../agent-sidecar/sidecar.mjs");
    if !std::path::Path::new(script).exists() {
        return Err(format!("answer engine script not found at {script} (was the project folder moved?)"));
    }
    let mut command = Command::new(&node);
    if let Some(dir) = std::path::Path::new(&node).parent().filter(|d| !d.as_os_str().is_empty()) {
        // An app opened from Finder has a bare PATH; the SDK's own process may look for node too.
        let path = std::env::var("PATH").unwrap_or_default();
        command.env("PATH", format!("{}:{path}", dir.display()));
    }
    let mut child = command
        .arg(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("could not start the answer engine (is Node.js installed?): {e}"))?;
    let mut stdin = child.stdin.take().ok_or("no stdin on agent process")?;
    let stdout = child.stdout.take().ok_or("no stdout on agent process")?;

    let retired = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (app2, pending2, translate_flags2, retired2) = (app.clone(), pending.clone(), translate_flags.clone(), retired.clone());
    std::thread::spawn(move || read_events(app2, stdout, pending2, translate_flags2, retired2));

    write_line(
        &mut stdin,
        &serde_json::json!({ "type": "start", "system": system, "model": model }),
    )?;
    Ok(Agent { retired, child, stdin, system: system.to_string(), model: model.to_string() })
}

/// Makes sure a live session exists with this system prompt and model. A new
/// system prompt (context or language changed) needs a fresh session;
/// a model change is switched in place.
fn ensure_session(app: &AppHandle, state: &AgentState, system: &str, model: &str) -> Result<(), String> {
    let mut guard = state.agent.lock().unwrap();
    if let Some(agent) = guard.as_mut() {
        let alive = matches!(agent.child.try_wait(), Ok(None));
        if alive && agent.system == system {
            if agent.model != model {
                write_line(&mut agent.stdin, &serde_json::json!({ "type": "set_model", "model": model }))?;
                agent.model = model.to_string();
            }
            return Ok(());
        }
    }
    // Requests still waiting on the old process can never be answered: fail them now,
    // so the old reader thread doesn't have to (it would also hit the next request).
    let old_waiting: Vec<_> = state.pending.lock().unwrap().drain().collect();
    for (_, tx) in old_waiting {
        let _ = tx.send(Err("answer engine restarted".to_string()));
    }
    *guard = None; // drops (kills) any old process
    *guard = Some(spawn_agent(app, &state.pending, &state.translate_flags, system, model)?);
    Ok(())
}

/// Routing hint prepended to a question, chosen by keywords. Measured
/// 2026-09-19 (Haiku): "why do you want to work at our company" got the
/// "why are you leaving" entry in 5 of 12 runs when the model decided alone.
/// A keyword check costs nothing and cannot drift between runs.
fn question_hint(text: &str, banned: &[String]) -> Option<String> {
    let q = text.to_lowercase();
    const LEAVING: &[&str] = &[
        "leaving", "leave your", "left your", "new job", "looking for a new",
        "уходите", "уход", "уйти", "ищете новую", "ищете работу",
    ];
    const COMPANY: &[&str] = &[
        "work here", "work at our", "work for us", "work with us", "join us",
        "join our", "our company", "your company", "why us", "about us",
        "about our", "this role", "this position", "this company",
        "interested in us", "interested in our", "у нас", "к нам", "нашей компании",
        "нашу компанию", "эту роль", "эту позицию", "о нас", "нашем",
    ];
    const EXPERIENCE: &[&str] = &[
        "experience with", "experience in", "experience using", "your experience",
        "how long have you", "years of experience", "ваш опыт", "опыт работы с",
        "опыт с ",
    ];
    const PROD_PLACE: &[&str] = &["production", "продакшен", "в проде", "на проде"];
    const PROD_TROUBLE: &[&str] = &[
        "bug", "incident", "issue", "outage", "баг", "инцидент", "авари", "проблем", "ошибк",
    ];
    let prod_incident = PROD_PLACE.iter().any(|k| q.contains(k)) && PROD_TROUBLE.iter().any(|k| q.contains(k));
    if LEAVING.iter().any(|k| q.contains(k)) {
        Some("[Routing hint: this question is about the candidate's CURRENT job. Never repeat this line.]".to_string())
    } else if prod_incident {
        Some("[Routing hint: production bug/incident question. Give ONLY the prepared text of the profile entry about a production incident, as is, without any added example, story, company name or project. Never write Time/Space lines. Never repeat this line.]".to_string())
    } else if COMPANY.iter().any(|k| q.contains(k)) {
        Some("[Routing hint: this question is about the interviewer's company and this role, NOT the current job. Answer from the company card entry. Never repeat this line.]".to_string())
    } else if EXPERIENCE.iter().any(|k| q.contains(k)) {
        let ban = if banned.is_empty() {
            "Do not describe".to_string()
        } else {
            format!("Never write the words {}, and do not describe", banned.join(", "))
        };
        Some(format!("[Routing hint: skills question. Answer ONLY by naming technologies, roles and projects that are written in the candidate's profile entries, in 2-4 short sentences. {ban} scale, performance or architecture unless a profile entry states exactly that. Never repeat this line.]"))
    } else {
        None
    }
}

/// Facts reminder added to EVERY question, right next to it: on opinion questions
/// ("how do you decide REST vs GraphQL") the model tacked on an invented "my experience"
/// line (eval 2026-09-19, "REST for high-throughput payment flows"). Rules that sit at the
/// end of a 100 KB system prompt are followed less often than a line next to the question.
fn facts_reminder(banned: &[String]) -> String {
    let ban = if banned.is_empty() {
        String::new()
    } else {
        format!(" Never write {}, or claims about payment flows or scale.", banned.join(", "))
    };
    format!("[Rule: whenever you say anything about the candidate's own experience, use only facts written in the profile entries.{ban} Never repeat this line.]")
}

/// Built-in banned words, used for a profile without its own `BANNED-WORDS.txt`.
const DEFAULT_BANNED: &[&str] = &["high-traffic", "high-load", "high-throughput", "scalable", "microservices", "real traffic"];

fn banned_for(profile: &str) -> Vec<String> {
    crate::knowledge::banned_words(profile)
        .unwrap_or_else(|| DEFAULT_BANNED.iter().map(|w| w.to_string()).collect())
}

/// The user message actually sent: routing hint (if any), facts reminder, question.
fn prepare_question(text: &str, profile: &str) -> String {
    let banned = banned_for(profile);
    let reminder = facts_reminder(&banned);
    match question_hint(text, &banned) {
        Some(hint) => format!("{hint}\n{reminder}\n{text}"),
        None => format!("{reminder}\n{text}"),
    }
}

/// Starts the sidecar at app launch so the first question doesn't pay the
/// ~2s cold start. Uses the saved settings; if the request's prompt later
/// differs (e.g. a non-English answer language) the session is reopened then.
pub fn prewarm(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let s = crate::settings::load_settings(app.clone());
        let request = AnswerRequest {
            model: s.model.clone(),
            interview_context: s.interview_context.clone(),
            response_language: if s.stt_language == "en" { "English".into() } else { String::new() },
            text: String::new(),
            image_data_uri: None,
            company: Some(s.company.clone()),
            profile: Some(s.profile.clone()),
            translate_question: false,
        };
        let state = app.state::<AgentState>();
        if let Err(e) = ensure_session(&app, &state, &build_system_prompt(&request), &sanitize_model(&s.model)) {
            eprintln!("agent prewarm failed: {e}");
        }
    });
}

#[tauri::command]
pub async fn generate_answer(
    app: AppHandle,
    state: tauri::State<'_, AgentState>,
    request: AnswerRequest,
) -> Result<(), String> {
    let model = sanitize_model(&request.model);
    let system = build_system_prompt(&request);

    let text = if request.text.trim().is_empty() {
        "Answer the question in the attached image.".to_string()
    } else {
        request.text.trim().to_string()
    };
    let text = prepare_question(&text, request.profile.as_deref().unwrap_or(""));
    let image = match &request.image_data_uri {
        Some(uri) => {
            let (media_type, data) = split_data_uri(uri)?;
            Some(serde_json::json!({ "media_type": media_type, "data": data }))
        }
        None => None,
    };

    let (tx, rx) = oneshot::channel();
    let id = state.next_id.fetch_add(1, Ordering::Relaxed) + 1;
    {
        ensure_session(&app, &state, &system, &model).inspect_err(|message| {
            let _ = app.emit("llm-error", StreamError { message: message.clone() });
        })?;
        state.pending.lock().unwrap().insert(id, tx);
        let mut guard = state.agent.lock().unwrap();
        let agent = guard.as_mut().ok_or("answer engine is not running")?;
        let mut ask = serde_json::json!({ "type": "ask", "id": id, "text": text });
        if let Some(image) = image {
            ask["image"] = image;
        }
        // Pushed right before the `ask` line goes out, so its position in the
        // queue matches the order `read_events` will see the resulting deltas
        // in (see `TranslateFlags`).
        state.translate_flags.lock().unwrap().push_back(request.translate_question);
        if let Err(message) = write_line(&mut agent.stdin, &ask) {
            state.pending.lock().unwrap().remove(&id);
            state.translate_flags.lock().unwrap().pop_back();
            let _ = app.emit("llm-error", StreamError { message: message.clone() });
            return Err(message);
        }
    }

    rx.await.map_err(|_| "answer engine stopped".to_string())?
}


#[cfg(test)]
mod tests {
    use super::*;

    fn default_banned() -> Vec<String> {
        DEFAULT_BANNED.iter().map(|w| w.to_string()).collect()
    }


    #[test]
    fn plain_answer_is_untouched_and_billed() {
        let (off_topic, visible) = split_no_task_marker("Use a hash map to ");
        assert!(!off_topic);
        assert_eq!(visible, "Use a hash map to ");
    }

    #[test]
    fn marker_is_stripped_with_its_separator() {
        for head in [
            "NO_TASK_FOUND This isn't an interview question.",
            "NO_TASK_FOUND: This isn't an interview question.",
            "NO_TASK_FOUND — This isn't an interview question.",
            "NO_TASK_FOUND\nThis isn't an interview question.",
            // Leading whitespace before the marker is common enough to allow.
            "\n\nNO_TASK_FOUND This isn't an interview question.",
        ] {
            let (off_topic, visible) = split_no_task_marker(head);
            assert!(off_topic, "should be a refusal: {head:?}");
            assert_eq!(visible, "This isn't an interview question.");
        }
    }

    /// The marker is English and capitalised regardless of the interview
    /// language, so a non-English refusal must still be recognised.
    #[test]
    fn marker_is_recognised_ahead_of_a_translated_sentence() {
        let (off_topic, visible) = split_no_task_marker("NO_TASK_FOUND Это не вопрос с интервью.");
        assert!(off_topic);
        assert_eq!(visible, "Это не вопрос с интервью.");
    }

    /// An answer that merely *mentions* the token mid-sentence is a real
    /// answer — only a prefix counts, or a model quoting these docs back
    /// would hand out free hints.
    #[test]
    fn marker_only_counts_as_a_prefix() {
        let (off_topic, visible) = split_no_task_marker("The app looks for NO_TASK_FOUND here.");
        assert!(!off_topic);
        assert_eq!(visible, "The app looks for NO_TASK_FOUND here.");
    }

    /// The head is only inspected once it is at least as long as the marker,
    /// so a partial marker must not read as a refusal — it would strip
    /// nothing and bill nothing while the rest of the sentence streamed on.
    #[test]
    fn routing_hint_separates_company_and_current_job_questions() {
        for q in [
            "Why do you want to work at our company?",
            "Why do you want to work here?",
            "Why are you interested in this role?",
            "What do you know about us?",
            "Почему вы хотите работать у нас?",
        ] {
            assert!(question_hint(q, &default_banned()).unwrap().contains("interviewer's company"), "{q}");
        }
        for q in ["Why are you leaving your current job?", "Почему вы уходите с текущей работы?", "Why are you looking for a new job?"] {
            assert!(question_hint(q, &default_banned()).unwrap().contains("CURRENT job"), "{q}");
        }
        for q in ["How would you describe your experience with Node.js?", "Расскажите про ваш опыт работы с React"] {
            assert!(question_hint(q, &default_banned()).unwrap().contains("skills question"), "{q}");
        }
        for q in [
            "Tell me about a bug you found in production.",
            "Describe a production issue you had to fix.",
            "Tell me about a production incident you handled.",
            "Расскажите о баге, который вы нашли в продакшене.",
        ] {
            assert!(question_hint(q, &default_banned()).unwrap().contains("production bug/incident"), "{q}");
        }
        for q in ["How do you take a feature to production?", "What are you looking for in your next role?", "Tell me about yourself.", "What is a closure?"] {
            assert!(question_hint(q, &default_banned()).is_none(), "{q}");
        }
    }

    #[test]
    fn partial_marker_is_not_a_refusal() {
        let (off_topic, _) = split_no_task_marker("NO_TASK_FOU");
        assert!(!off_topic);
    }

    /// Writes `{question: full user message}` for every question in `JH_QUESTIONS` (a JSON
    /// array of objects with a `q` field) to `JH_HINTS`, so the prompt test in
    /// agent-sidecar/eval uses the app's own routing rules, not a copy.
    #[test]
    #[ignore]
    fn dump_hints_for_eval() {
        let src = std::fs::read_to_string(std::env::var("JH_QUESTIONS").expect("JH_QUESTIONS")).unwrap();
        let items: Vec<serde_json::Value> = serde_json::from_str(&src).unwrap();
        let mut map = serde_json::Map::new();
        for item in &items {
            let q = item["q"].as_str().unwrap();
            map.insert(q.to_string(), prepare_question(q, &std::env::var("JH_PROFILE").unwrap_or_default()).into());
        }
        std::fs::write(std::env::var("JH_HINTS").expect("JH_HINTS"), serde_json::to_string(&map).unwrap()).unwrap();
    }

    #[test]
    #[ignore]
    fn dump_prompt_for_manual_check() {
        let req = AnswerRequest {
            model: "claude-haiku-4-5".into(),
            interview_context: String::new(),
            response_language: "English".into(),
            text: String::new(),
            image_data_uri: None,
            company: std::env::var("JH_COMPANY").ok(),
            profile: std::env::var("JH_PROFILE").ok(),
            translate_question: false,
        };
        let path = std::env::var("JH_DUMP").expect("JH_DUMP");
        std::fs::write(path, build_system_prompt(&req)).unwrap();
    }
}

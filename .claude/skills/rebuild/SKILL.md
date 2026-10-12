---
name: rebuild
description: Пересобрать приложение Notepad, заменить его в /Applications и удалить старые копии. Использовать после правок кода приложения (базе знаний пересборка не нужна — достаточно Reload knowledge).
---

Пересобери и установи приложение. Команды — из папки проекта. Перед началом скажи, что будешь делать, и получи «да» (закрывается запущенное приложение, заменяется копия в `/Applications`).

1. Закрыть: `pkill -x notepad; sleep 1`
2. Собрать: `cd app && pnpm tauri build --bundles app 2>&1 | tail -25`. Упала сборка — покажи ошибку, ничего не удаляй. (Если у пользователя есть своя подпись, она задаётся через `APPLE_SIGNING_IDENTITY`; нет — собирается без неё, права на микрофон и запись экрана придётся выдать заново.)
3. Удалить старые копии: `rm -rf "/Applications/Notepad.app" "$HOME/Applications/Notepad.app"`
4. Установить: `cp -R app/src-tauri/target/release/bundle/macos/Notepad.app /Applications/`
5. Убрать копию из папки сборки: `rm -rf app/src-tauri/target/release/bundle/macos/Notepad.app`
6. Проверить, что копия одна: `mdfind "kMDItemFSName == 'Notepad.app'"`. Видишь лишнее — покажи путь и спроси, удалять ли.

В конце одной строкой: собрано, установлено, сколько копий.

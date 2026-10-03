# IME precheck retry

System Settings AX and screenshots were available. Temporary Pinyin – Simplified
and the input menu were enabled. Keyboard Shortcuts confirmed enabled Control-Space
(previous source) and Control-Option-Space (next source). A Control-Space followed
by separate `pressKey("n")` and `pressKey("i")` calls in Settings search produced
plain `ni`, without an observed candidate window or marked text. Binding the input
menu agent and SystemUIServer both timed out; explicit menu selection could not
be verified. No native app was running. This is inconclusive infrastructure/input
selection evidence, not a GPUI composition failure or pass.

ABC-only sources, hidden input menu and English (United States)-only dictation
were restored through Settings. The automatically added Mandarin dictation
selection was removed. Settings quit normally. No keyboard bindings were changed.
VoiceOver was not enabled. Tool-tab real IME acceptance remains pending.

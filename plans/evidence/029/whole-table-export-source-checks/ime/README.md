# Caps Lock activation preparation

System Settings was available. The agent temporarily added built-in Pinyin and
enabled the visible “Use the Caps Lock key to switch to and from ABC” option.
The search field was explicitly focused with AX setValue. Separate physical
Caps_Lock, n and i events produced plain `ni`; toggling Caps_Lock again and
pressing n, i and Space produced plain `ni `. No marked text or candidate window
was observed. This is inconclusive input-source activation evidence before
native-window interaction, not a GPUI IME pass or failure.

The Caps Lock switch was returned to off, Pinyin removed, the input menu hidden,
and the automatically added Mandarin dictation selection removed. The recorded
restoration states show ABC only and English (United States) only. System
Settings quit normally. VoiceOver was unchanged. Real Tool-tab IME remains open;
the earlier scoped SQL/form/cell composition evidence remains separate.

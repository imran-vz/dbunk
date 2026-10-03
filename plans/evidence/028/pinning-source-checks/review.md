# Independent pinning source review

Read-only review covered source/display mapping through copy, edit and auto-fit;
frozen-pane geometry and virtualization; per-result ownership; retained-payload
admission; and latest-record table preference merging.

The reviewer found that an all-hidden recovery fallback could accept a pin yet
continue to render unpinned. The implementation now explicitly requires Show all
columns before pinning or moving that fallback, preserving stored preferences.
A focused test covers refusal, unchanged storage, showing all, and unpinning.
The reviewer rechecked this and the concurrent visible-neighbor guard and reported
no remaining concrete findings. The review did not run tests or operate UI or
fixtures. Actual-window evidence is separately required.

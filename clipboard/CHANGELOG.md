# Changelog

## 0.2.1 — 2026-11-07

- Fixed: copying again in an app that already owned the clipboard often lost the
  new copy, and pasting gave nothing, so you had to copy twice. Apps that replace
  their copy drop the old one first, which empties the clipboard for a moment;
  clipboard took over in that moment and its old copy won the race. It now waits
  until the clipboard has stayed empty for 100 ms.

## 0.2.0 — 2026-11-03

- Arch Linux builds. No change in behavior.

## 0.1.1 — 2026-11-02

- Runs only once per session: a second start exits instead of fighting the first.

## 0.1.0 — 2026-11-02

- First release. Keeps the last copied text and image (PNG preferred) after the
  app that copied them closes. Copies stream through memory files without passing
  through the daemon's heap; idle costs no CPU. Copies marked secret by password
  managers are never kept.

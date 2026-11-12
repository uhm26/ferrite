# auth

A polkit authentication agent for Wayland: the window that asks for your password when an
app needs more rights, such as mounting an internal disk in a file manager or `pkexec`.

- Registers with polkit as the agent of your session and stays running. Start it once per
  session, e.g. from your compositor's autostart.
- Shows what is asked for, which program asks, and whose password it wants.
- Never checks the password itself: it hands what you type to polkit's own helper, which
  does the check. A wrong password shows an error and lets you try again.
- Built on the workspace's [`frame`](../frame) library, in the same look as the other tools.

## Keys

| Key | Does |
|---|---|
| Enter | Authenticate |
| Esc | Cancel |
| Backspace | Delete one character; with Ctrl, clear the field |
| Ctrl+U | Clear the field |

## Requirements

polkit 126 or newer with `polkit-agent-helper.socket` (Arch's and Fedora's polkit ship it), a Wayland
compositor, and `XDG_SESSION_ID` set in the environment (logind sets it at login).

## License

MIT or Apache-2.0, see the [main README](../README.md#license).

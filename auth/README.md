# auth

A polkit authentication agent for Wayland: the window that asks for your password when an
app needs more rights, such as mounting an internal disk in a file manager.

**Not built yet.** This folder only holds the plan's skeleton.

Planned:

- Registers with polkit as the agent of your session and stays running.
- Shows what is being asked for and by which program, plus a password field.
- Never checks the password itself: it hands what you type to polkit's own helper
  (`polkit-agent-helper-1`), which does the check.
- Built on the workspace's [`frame`](../frame) library, in the same look as the other tools.

## License

MIT or Apache-2.0, see the [main README](../README.md#license).

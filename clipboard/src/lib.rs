//! clip: a Wayland clipboard keeper. It reads every text copy as it happens and,
//! when the copying app closes and the clipboard goes empty, serves the copy itself.
//! One thread, one poll() loop, no crates, no timers: idle means asleep.

pub mod daemon;
pub mod keeper;
pub mod pipes;
pub mod sys;
pub mod wl;

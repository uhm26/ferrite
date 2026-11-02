//! Widgets and the look (docs/decisions/001, 002): [`theme`] tokens from the ui
//! skill (flat, boltsnap-inspired), [`paint`] antialiased shapes, [`text`] cached
//! subpixel glyphs in four weights, [`icon`] our own line icons, [`motion`] springs
//! and tweens, [`ctx`] the immediate-mode frame, [`widgets`] the components,
//! [`field`] the text field (caret, selection, mouse), [`menu`] context menus,
//! [`shell`] the app window's frame (folding sidebar, toolbar and status bands, pane).
//! `examples/gallery` shows all of it.

pub mod ctx;
pub mod field;
pub mod icon;
pub mod menu;
pub mod motion;
pub mod paint;
pub mod shell;
pub mod text;
pub mod theme;
pub mod widgets;

pub use ctx::{Input, State, Ui};
pub use icon::Icon;
pub use menu::{MenuEvent, MenuHot, MenuItem};
pub use paint::Rect;
pub use shell::{Shell, SideItem, Sidebar};
pub use text::{Ink, Text};
pub use widgets::Align;

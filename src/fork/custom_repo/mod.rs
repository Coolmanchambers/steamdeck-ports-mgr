//! "Use a GitHub link...": a port that is not in the catalog, added from its GitHub page.
//!
//! - `parse`: what the user typed or pasted becomes `owner/project`, or a plain-words reason it cannot be.
//!
//! The check of the newest release and the catalog entry come in later parts; nothing here touches the
//! network or any file.

pub mod parse;

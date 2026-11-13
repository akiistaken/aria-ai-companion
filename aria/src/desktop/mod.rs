//! Everything Aria knows about, and does to, the Windows desktop.
//!
//! | Module | What |
//! |---|---|
//! | [`win32`] | windows, processes, monitors, regions, screen capture, job objects |
//! | [`input`] | synthetic keyboard and mouse, cursor and button state |
//! | [`cua`] | Cua Driver over MCP: background clicks and typing, accessibility trees, window lists |
//! | [`presence`] | at the desk or away; is a game in the foreground |
//! | [`media`] | what is playing (system media sessions) |
//! | [`beat`] | tempo and beat phase of the music, for dancing in time |
//! | [`spotify`] | verified Spotify control and media keys |
//! | [`audio`] | per-app audio meters, loopback capture, devices |
//! | [`system`] | CPU, RAM, disk, battery |
//! | [`launcher`] | finding and opening apps, sites and URIs |

pub mod audio;
pub mod beat;
pub mod com;
pub mod cua;
pub mod input;
mod game_catalog;
pub mod launcher;
pub mod media;
pub mod presence;
pub mod spotify;
pub mod system;
pub mod win32;

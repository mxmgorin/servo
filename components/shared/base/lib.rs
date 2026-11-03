/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

#![deny(unsafe_code)]

//! A crate to hold very common types in Servo.
//!
//! You should almost never need to add a data type to this crate. Instead look for
//! a more shared crate that has fewer dependents.

pub mod cross_process_instant;
pub mod generic_channel;
#[macro_use]
pub mod id;
pub mod print_tree;
mod rope;
pub mod text;
pub mod threadboost;
pub mod threadpool;
mod unicode_block;

use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::Path;

use log::{trace, warn};
use malloc_size_of_derive::MallocSizeOf;
pub use rope::{Rope, RopeChars, RopeIndex, RopeMovement, RopeSlice};
use serde::{Deserialize, Serialize};
use webrender_api::Epoch as WebRenderEpoch;

pub fn read_json_from_file<T>(data: &mut T, config_dir: &Path, filename: &str)
where
    T: for<'de> Deserialize<'de>,
{
    let path = config_dir.join(filename);
    let display = path.display();

    let file = match File::open(&path) {
        Err(why) => {
            warn!("couldn't open {}: {}", display, why);
            return;
        },
        Ok(file) => file,
    };
    let buf_reader = BufReader::new(file);

    match serde_json::from_reader(buf_reader) {
        Ok(decoded_buffer) => *data = decoded_buffer,
        Err(why) => warn!("Could not deserialize file {}", why),
    }
}

/// Writes `data` as JSON to `filename` in `config_dir`, replacing the file only once
/// the whole write succeeded. Failures are logged.
pub fn write_json_to_file<T>(data: &T, config_dir: &Path, filename: &str)
where
    T: Serialize,
{
    let path = config_dir.join(filename);
    let partial = config_dir.join(format!("{filename}.partial"));
    let write = || -> std::io::Result<()> {
        let mut writer = BufWriter::new(File::create(&partial)?);
        serde_json::to_writer_pretty(&mut writer, data)?;
        writer.flush()?;
        std::fs::rename(&partial, &path)
    };
    match write() {
        Ok(()) => trace!("successfully wrote to {}", path.display()),
        Err(why) => warn!("couldn't write {}: {}", path.display(), why),
    }
}

/// A struct for denoting the age of messages; prevents race conditions.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Deserialize,
    Eq,
    Hash,
    Ord,
    PartialEq,
    PartialOrd,
    Serialize,
    MallocSizeOf,
)]
pub struct Epoch(pub u32);

impl Epoch {
    pub fn next(&self) -> Self {
        Self(self.0 + 1)
    }
}

impl From<Epoch> for WebRenderEpoch {
    fn from(val: Epoch) -> Self {
        WebRenderEpoch(val.0)
    }
}

pub trait WebRenderEpochToU16 {
    fn as_u16(&self) -> u16;
}

impl WebRenderEpochToU16 for WebRenderEpoch {
    /// The value of this [`Epoch`] as a u16 value. Note that if this Epoch's
    /// value is more than u16::MAX, then the return value will be modulo
    /// u16::MAX.
    fn as_u16(&self) -> u16 {
        (self.0 % u16::MAX as u32) as u16
    }
}

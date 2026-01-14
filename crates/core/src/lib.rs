pub mod codec;
pub mod error;
pub mod format;
pub mod io;
pub mod list;
pub mod model;
pub mod pack;
pub mod unpack;
pub mod utils;

pub use error::DzipError;
pub use io::{
    PackSink, PackSource, ReadSeekSend, UnpackSink, UnpackSource, WriteSeekSend, WriteSend,
};
pub use list::{ListEntry, do_list};
pub use pack::do_pack;
pub use unpack::do_unpack;

pub type Result<T> = std::result::Result<T, DzipError>;

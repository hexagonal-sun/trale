//! Async filesystem operations.
//!
//! This module implements an async version of [std::fs]. It allows files to be
//! opened, read from and written to asynchronously.
//!
//! # Example
//!
//! Here is an example of creating a new file and writing the string "Hello,
//! world!", seeking to the start and reading it back into a buffer.
//!
//! ```
//! use trale::task::Executor;
//! use trale::futures::fs::File;
//! use trale::futures::write::AsyncWrite;
//! use crate::trale::futures::read::AsyncRead;
//! use std::io::Seek;
//!# use assert_fs::TempDir;
//!# use assert_fs::fixture::PathChild;
//!
//! Executor::block_on(async {
//!#     let dir = TempDir::new().unwrap();
//!#     let path = dir.child("test.txt").to_path_buf();
//!     let mut buf = [0; 1024];
//!     let mut file = File::create(path).await?;
//!     file.write("Hello, world!".as_bytes()).await?;
//!
//!     file.rewind()?;
//!
//!     let len = file.read(&mut buf).await?;
//!     assert_eq!(&buf[..len], "Hello, world!".as_bytes());
//!#     Ok::<(), std::io::Error>(())
//! });
//! ```
use std::{
    ffi::CString,
    future::Future,
    io::{self, Result, Seek},
    os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd},
    path::Path,
};

use io_uring::{opcode, types};
use libc::{O_CREAT, O_RDWR};

use crate::reactor::{Reactor, ReactorIo};

use super::{
    read::{AsyncRead, AsyncReader},
    write::{AsyncWrite, AsyncWriter},
};

/// An open file.
///
/// This object represents a file that has been opened by one of the
/// [File::create] or [File::open] methods. It can be used for reading and
/// writing data via the [File::read] and [File::write] functions respectively.
pub struct File {
    inner: OwnedFd,
}

/// A future for creating a directory.
///
/// This future can be `.await`ed in order to create a directory. If the
/// directory could not be created an `Err` value is returned with the
/// underlying error indicating the reason for failure.
pub struct Mkdir {
    io: ReactorIo,
    path: CString,
}

/// A future for opening a file.
///
/// This future can be `.await`ed in order to obtain an open [File] object. It
/// is created by one of the [File::open] or [File::create] methods. Note that
/// if the file could not be opened and/or created an `Err` value is returned
/// with the underlying error indicating the reason for failure.
pub struct FileOpen {
    path: CString,
    flags: i32,
    io: ReactorIo,
}

impl Future for FileOpen {
    type Output = Result<File>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };

        this.io
            .submit_or_get_result(|| {
                (
                    opcode::OpenAt::new(types::Fd(libc::AT_FDCWD), this.path.as_ptr())
                        .flags(this.flags)
                        .mode(0o777)
                        .build(),
                    cx.waker().clone(),
                )
            })
            .map(|x| {
                x.map(|x| File {
                    inner: unsafe { OwnedFd::from_raw_fd(x) },
                })
            })
    }
}

impl Future for Mkdir {
    type Output = Result<()>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };

        this.io
            .submit_or_get_result(|| {
                (
                    opcode::MkDirAt::new(types::Fd(libc::AT_FDCWD), this.path.as_ptr())
                        .mode(0o777)
                        .build(),
                    cx.waker().clone(),
                )
            })
            .map(|x| x.map(|_| ()))
    }
}
impl File {
    /// Attempt to open an existing file.
    ///
    /// This function takes a path to an already existant path and returns a
    /// future which attempts to open it in read/write mode. If the path does
    /// not exist `.await`ing the returned [FileOpen] future will yield an
    /// error.
    pub fn open(path: impl AsRef<Path>) -> FileOpen {
        FileOpen {
            path: CString::new(path.as_ref().as_os_str().as_encoded_bytes()).unwrap(),
            flags: O_RDWR,
            io: Reactor::new_io(),
        }
    }

    /// Attempt to create a new file.
    ///
    /// This function takes a path and returns a future which attempts to create
    /// it if it does not exist. If the path already exists, the file is opened.
    /// In both cases, the file is opened in read/write mode.
    pub fn create(path: impl AsRef<Path>) -> FileOpen {
        FileOpen {
            path: CString::new(path.as_ref().as_os_str().as_encoded_bytes()).unwrap(),
            flags: O_RDWR | O_CREAT,
            io: Reactor::new_io(),
        }
    }

    /// Attempt to create a new directory.
    ///
    /// This function takes a path and returns a future which attempts to create
    /// the specified directory. If the path is relative, the base path is the
    /// CWD of the program.
    pub fn mkdir(path: impl AsRef<Path>) -> Mkdir {
        Mkdir {
            io: Reactor::new_io(),
            path: CString::new(path.as_ref().as_os_str().as_encoded_bytes()).unwrap(),
        }
    }
}

impl AsyncRead for File {
    fn read(&mut self, buf: &mut [u8]) -> impl Future<Output = io::Result<usize>> {
        AsyncReader {
            fd: self.inner.as_fd(),
            buf,
            io: Reactor::new_io(),
            seekable: true,
        }
    }
}

impl AsyncWrite for File {
    fn write(&mut self, buf: &[u8]) -> impl Future<Output = io::Result<usize>> {
        AsyncWriter {
            fd: self.inner.as_fd(),
            buf,
            io: Reactor::new_io(),
            seekable: true,
        }
    }
}

impl Seek for File {
    fn seek(&mut self, pos: io::SeekFrom) -> Result<u64> {
        let (off, whence) = match pos {
            io::SeekFrom::Start(off) => (off as i64, libc::SEEK_SET),
            io::SeekFrom::End(off) => (off, libc::SEEK_END),
            io::SeekFrom::Current(off) => (off, libc::SEEK_CUR),
        };

        let res = unsafe { libc::lseek(self.inner.as_raw_fd(), off, whence) };

        if res == -1 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(res as u64)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Seek;

    use assert_fs::{
        assert::PathAssert,
        prelude::{FileWriteStr, PathChild},
        TempDir,
    };

    use crate::{
        futures::{read::AsyncRead, write::AsyncWrite},
        task::Executor,
    };

    #[test]
    fn simple_write() {
        let dir = TempDir::new().unwrap();
        let child = dir.child("test.txt");
        let child_path = child.to_path_buf();

        Executor::block_on(async move {
            let mut f = super::File::create(child_path).await.unwrap();
            f.write("Hello, world!".as_bytes()).await.unwrap()
        });

        child.assert("Hello, world!");
    }

    #[test]
    fn simple_read() {
        let dir = TempDir::new().unwrap();
        let child = dir.child("test.txt");
        child.write_str("Hello, async!").unwrap();

        let child_path = child.to_path_buf();

        Executor::block_on(async move {
            let mut f = super::File::open(child_path).await.unwrap();
            let mut buf = [0; 1024];
            let len = f.read(&mut buf).await.unwrap();

            assert_eq!(&buf[..len], "Hello, async!".as_bytes());
        });
    }

    #[test]
    fn no_file_open_error() {
        let dir = TempDir::new().unwrap();
        let child = dir.child("test.txt");
        let child_path = child.to_path_buf();

        Executor::block_on(async move {
            let f = super::File::open(child_path).await;

            assert!(f.is_err());
        });
    }

    #[test]
    fn create_on_existing_file() {
        let dir = TempDir::new().unwrap();
        let child = dir.child("test.txt");
        child.write_str("data").unwrap();
        let child_path = child.to_path_buf();

        Executor::block_on(async move {
            let f = super::File::create(child_path).await;

            assert!(f.is_ok());
        });

        child.assert("data");
    }

    #[test]
    fn consecutive_writes() {
        let dir = TempDir::new().unwrap();
        let child = dir.child("test.txt");
        let child_path = child.to_path_buf();

        Executor::block_on(async move {
            let mut f = super::File::create(child_path).await.unwrap();

            f.write("Hello".as_bytes()).await.unwrap();

            f.write("ABCD".as_bytes()).await.unwrap();
        });

        child.assert("HelloABCD");
    }

    #[test]
    fn consecutive_reads() {
        let dir = TempDir::new().unwrap();
        let child = dir.child("test.txt");
        child.write_str("abcdef").unwrap();
        let child_path = child.to_path_buf();

        Executor::block_on(async move {
            let mut buf = [0; 2];
            let mut f = super::File::open(child_path).await.unwrap();

            f.read(&mut buf).await.unwrap();
            assert_eq!(buf, "ab".as_bytes());

            f.read(&mut buf).await.unwrap();
            assert_eq!(buf, "cd".as_bytes());

            f.read(&mut buf).await.unwrap();
            assert_eq!(buf, "ef".as_bytes());
        });
    }

    #[test]
    fn simple_seek() {
        let dir = TempDir::new().unwrap();
        let child = dir.child("test.txt");
        child.write_str("abcdef").unwrap();
        let child_path = child.to_path_buf();

        Executor::block_on(async move {
            let mut buf = [0; 2];
            let mut f = super::File::open(child_path).await.unwrap();

            f.read(&mut buf).await.unwrap();
            assert_eq!(buf, "ab".as_bytes());

            f.read(&mut buf).await.unwrap();
            assert_eq!(buf, "cd".as_bytes());

            f.read(&mut buf).await.unwrap();
            assert_eq!(buf, "ef".as_bytes());

            f.seek(std::io::SeekFrom::Current(-4)).unwrap();

            f.read(&mut buf).await.unwrap();
            assert_eq!(buf, "cd".as_bytes());

            f.read(&mut buf).await.unwrap();
            assert_eq!(buf, "ef".as_bytes());
        });
    }

    #[test]
    fn simple_mkdir() {
        let dir = TempDir::new().unwrap();
        let dir_path = dir.to_path_buf();

        Executor::block_on(async move {
            super::File::mkdir(dir_path.join("test_dir")).await.unwrap();
        });

        assert!(dir.child("test_dir").is_dir());
    }
}

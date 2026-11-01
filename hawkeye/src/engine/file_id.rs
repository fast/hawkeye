// Copyright 2026 FastLabs Developers
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::fs;
use std::io;
use std::path::Path;

#[cfg(unix)]
pub type FileId = (u64, u64);

#[cfg(unix)]
pub fn get_file_id(path: &Path) -> io::Result<FileId> {
    use std::os::unix::fs::MetadataExt;

    let metadata = fs::metadata(path)?;
    Ok((metadata.dev(), metadata.ino()))
}

#[cfg(windows)]
#[derive(Debug, PartialEq, Eq, Hash)]
pub enum FileId {
    Extended { volume: u64, id: [u8; 16] },
    Legacy { volume: u32, index: u64 },
}

#[cfg(windows)]
#[expect(unsafe_code, reason = "Windows file identity requires Win32 calls")]
pub fn get_file_id(path: &Path) -> io::Result<FileId> {
    use std::mem::size_of;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION;
    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS;
    use windows_sys::Win32::Storage::FileSystem::FILE_ID_INFO;
    use windows_sys::Win32::Storage::FileSystem::FileIdInfo;
    use windows_sys::Win32::Storage::FileSystem::GetFileInformationByHandle;
    use windows_sys::Win32::Storage::FileSystem::GetFileInformationByHandleEx;

    // Query metadata without requiring read access to file contents. Opening follows symlinks.
    let file = fs::OpenOptions::new()
        .access_mode(0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)?;
    // ReFS needs the full 128-bit identifier; its legacy 64-bit index is not guaranteed unique.
    let mut info = FILE_ID_INFO::default();
    // SAFETY: The handle remains open, and the writable buffer and its size match FileIdInfo.
    let success = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&mut info as *mut FILE_ID_INFO).cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if success != 0 {
        return Ok(FileId::Extended {
            volume: info.VolumeSerialNumber,
            id: info.FileId.Identifier,
        });
    }

    // Some filesystems do not support FileIdInfo. Keep the legacy identity in a separate
    // variant so its 64-bit index cannot be confused with an extended 128-bit identifier.
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: The handle remains open, and info is a writable buffer of the required type.
    let success = unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) };
    if success == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(FileId::Legacy {
        volume: info.dwVolumeSerialNumber,
        index: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
    })
}

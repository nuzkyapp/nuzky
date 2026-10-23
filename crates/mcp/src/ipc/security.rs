use std::fs::{self, File, OpenOptions};
use std::io::Read;
#[cfg(windows)]
use std::os::windows::{
    ffi::OsStrExt,
    fs::{MetadataExt, OpenOptionsExt},
    io::{AsRawHandle, AsRawSocket, FromRawHandle, OwnedHandle},
};
#[cfg(unix)]
use std::os::{
    fd::AsRawFd,
    unix::{
        fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
        net::UnixStream,
    },
};
use std::path::Path;
#[cfg(windows)]
use std::ptr::{null, null_mut};

use anyhow::{Context, Result, ensure};
#[cfg(windows)]
use uds_windows::UnixStream;
#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{HANDLE, LocalFree},
    Networking::WinSock::{SIO_AF_UNIX_GETPEERPID, SOCKET, WSAIoctl},
    Security::{
        ACCESS_ALLOWED_ACE, ACL_SIZE_INFORMATION, AclSizeInformation,
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo,
            SDDL_REVISION_1, SE_FILE_OBJECT,
        },
        DACL_SECURITY_INFORMATION, GetAce, GetAclInformation, GetTokenInformation, OWNER_SECURITY_INFORMATION, PSID,
        SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
    },
    Storage::FileSystem::{
        CreateDirectoryW, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FileAttributeTagInfo, GetFileInformationByHandleEx,
        READ_CONTROL,
    },
    System::{
        SystemServices::{ACCESS_ALLOWED_ACE_TYPE, IO_REPARSE_TAG_AF_UNIX},
        Threading::{GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION},
    },
};

pub(super) const TOKEN_LEN: usize = 64;

#[cfg(unix)]
pub(super) fn uid() -> u32 {
    // POSIX geteuid has no failure mode and does not access caller memory.
    unsafe { libc::geteuid() }
}

/// This process's user as a string SID, which the endpoint's folder and every peer must have.
#[cfg(windows)]
pub(super) fn user() -> Result<String> {
    // The pseudo-handle of this process needs no closing.
    process_user(unsafe { GetCurrentProcess() })
}

#[cfg(windows)]
fn process_user(process: HANDLE) -> Result<String> {
    let mut token = null_mut();
    // OpenProcessToken writes a handle it opened, which OwnedHandle then closes.
    ensure!(unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } != 0, "IPC_UNTRUSTED: cannot read a user");
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    // TOKEN_USER and the SID it points to; u64s keep the pointer aligned.
    let mut buffer = [0u64; 64];
    let mut length = 0;
    // GetTokenInformation writes at most the buffer's size.
    let read = unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            buffer.as_mut_ptr().cast(),
            std::mem::size_of_val(&buffer) as u32,
            &mut length,
        )
    };
    ensure!(read != 0, "IPC_UNTRUSTED: cannot read a user");
    // The buffer now starts with a TOKEN_USER whose SID lies inside it.
    sid(unsafe { (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid }).context("IPC_UNTRUSTED: cannot read a user")
}

/// A SID as its canonical text, such as S-1-5-21-…; None when it is not valid.
#[cfg(windows)]
fn sid(sid: PSID) -> Option<String> {
    let mut text = null_mut();
    // ConvertSidToStringSidW allocates a NUL-terminated string, read and freed here.
    unsafe {
        if ConvertSidToStringSidW(sid, &mut text) == 0 {
            return None;
        }
        let length = (0..).take_while(|&i| *text.add(i) != 0).count();
        let string = String::from_utf16_lossy(std::slice::from_raw_parts(text, length));
        LocalFree(text.cast());
        Some(string)
    }
}

#[cfg(unix)]
pub(super) fn directory(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path).context("IPC_UNTRUSTED: checking directory")?;
    ensure!(
        meta.is_dir() && meta.uid() == uid() && meta.mode() & 0o7777 == 0o700,
        "IPC_UNTRUSTED: endpoint directory must be owned by this user with mode 0700"
    );
    Ok(())
}

/// As 0700 on Unix: owned by this user, not a link, and only this user in its access list.
#[cfg(windows)]
pub(super) fn directory(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path).context("IPC_UNTRUSTED: checking directory")?;
    ensure!(
        meta.is_dir() && meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0 && private(path)?,
        "IPC_UNTRUSTED: endpoint directory must be owned by this user and open only to this user"
    );
    Ok(())
}

#[cfg(windows)]
fn private(path: &Path) -> Result<bool> {
    let folder = OpenOptions::new()
        .access_mode(READ_CONTROL)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .context("IPC_UNTRUSTED: opening directory")?;
    let user = user()?;
    let (mut owner, mut acl, mut descriptor) = (null_mut(), null_mut(), null_mut());
    // GetSecurityInfo points owner and acl into the descriptor it allocates, freed below.
    let error = unsafe {
        GetSecurityInfo(
            folder.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            &mut acl,
            null_mut(),
            &mut descriptor,
        )
    };
    ensure!(error == 0, "IPC_UNTRUSTED: reading directory permissions");
    let mut size = ACL_SIZE_INFORMATION::default();
    // A missing access list lets everyone in. Each entry GetAce hands out lies inside the list.
    let private = !acl.is_null()
        && sid(owner).as_ref() == Some(&user)
        && unsafe {
            GetAclInformation(
                acl,
                (&mut size as *mut ACL_SIZE_INFORMATION).cast(),
                size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            )
        } != 0
        && (0..size.AceCount).all(|index| unsafe {
            let mut entry = null_mut();
            GetAce(acl, index, &mut entry) != 0 && {
                let entry = &*entry.cast::<ACCESS_ALLOWED_ACE>();
                u32::from(entry.Header.AceType) == ACCESS_ALLOWED_ACE_TYPE
                    && sid((&entry.SidStart as *const u32).cast_mut().cast()).as_ref() == Some(&user)
            }
        });
    unsafe { LocalFree(descriptor) };
    Ok(private)
}

/// A folder only this user may open, as `directory` checks, also when Nuzky runs as administrator, whose
/// new files would otherwise belong to the Administrators group.
#[cfg(windows)]
pub(super) fn create_directory(path: &Path) -> std::io::Result<()> {
    let user = user().map_err(std::io::Error::other)?;
    let text: Vec<u16> = format!("O:{user}D:P(A;OICI;FA;;;{user})").encode_utf16().chain([0]).collect();
    let mut descriptor = null_mut();
    // The descriptor it allocates is freed below.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            text.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            null_mut(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    // Both pointers are valid for the call.
    let created = unsafe { CreateDirectoryW(wide.as_ptr(), &attributes) } != 0;
    let error = std::io::Error::last_os_error();
    unsafe { LocalFree(descriptor) };
    if created { Ok(()) } else { Err(error) }
}

#[cfg(unix)]
pub(super) fn socket(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path).context("IPC_UNTRUSTED: checking socket")?;
    ensure!(meta.file_type().is_socket() && meta.uid() == uid(), "IPC_UNTRUSTED: socket must be owned by this user");
    Ok(())
}

#[cfg(windows)]
pub(super) fn socket(path: &Path) -> Result<()> {
    let file = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .context("IPC_UNTRUSTED: checking socket")?;
    let mut info = FILE_ATTRIBUTE_TAG_INFO::default();
    // Writes the one FILE_ATTRIBUTE_TAG_INFO it is given.
    let read = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileAttributeTagInfo,
            (&mut info as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
            size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
    };
    ensure!(read != 0 && info.ReparseTag == IO_REPARSE_TAG_AF_UNIX, "IPC_UNTRUSTED: endpoint must be a socket");
    Ok(())
}

#[cfg(unix)]
pub(super) fn token(path: &Path) -> Result<String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .context("IPC_UNTRUSTED: opening token without following symlinks")?;
    let meta = file.metadata().context("IPC_UNTRUSTED: checking token descriptor")?;
    ensure!(
        meta.is_file() && meta.uid() == uid() && meta.mode() & 0o7777 == 0o600 && meta.len() == TOKEN_LEN as u64,
        "IPC_UNTRUSTED: token must be a private 0600 regular file of exactly 64 bytes"
    );
    let mut bytes = Vec::with_capacity(TOKEN_LEN + 1);
    file.take((TOKEN_LEN + 1) as u64).read_to_end(&mut bytes).context("IPC_UNTRUSTED: reading token")?;
    ensure!(
        bytes.len() == TOKEN_LEN && bytes.iter().all(u8::is_ascii_hexdigit),
        "IPC_UNTRUSTED: token must contain 64 hex digits"
    );
    String::from_utf8(bytes).context("IPC_UNTRUSTED: invalid token encoding")
}

/// A private 0600 regular file of this user, read without following symlinks; None when missing.
#[cfg(unix)]
pub(super) fn read_private(path: &Path, max: u64) -> Result<Option<String>> {
    let file = match OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("IPC_UNTRUSTED: opening private file without following symlinks"),
    };
    let meta = file.metadata().context("IPC_UNTRUSTED: checking private file")?;
    ensure!(
        meta.is_file() && meta.uid() == uid() && meta.mode() & 0o7777 == 0o600 && meta.len() <= max,
        "IPC_UNTRUSTED: {} must be a small private 0600 file of this user",
        path.display()
    );
    let mut text = String::new();
    file.take(max).read_to_string(&mut text).context("IPC_UNTRUSTED: reading private file")?;
    Ok(Some(text))
}

#[cfg(unix)]
pub(super) fn create_token(path: &Path) -> Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_EXCL)
        .open(path)
        .context("IPC_UNTRUSTED: creating exclusive token file")
}

#[cfg(unix)]
pub(super) fn peer(stream: &UnixStream) -> Result<()> {
    verify_uid(peer_uid(stream)?)
}

#[cfg(unix)]
fn verify_uid(peer: u32) -> Result<()> {
    ensure!(peer == uid(), "IPC_UNTRUSTED: peer belongs to a different user");
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn peer_uid(stream: &UnixStream) -> Result<u32> {
    let mut cred = libc::ucred { pid: 0, uid: 0, gid: 0 };
    let mut length = std::mem::size_of_val(&cred) as libc::socklen_t;
    // The kernel writes at most length bytes into the correctly sized ucred.
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast(),
            &mut length,
        )
    };
    ensure!(
        rc == 0 && length as usize == std::mem::size_of_val(&cred),
        "IPC_UNTRUSTED: cannot verify peer credentials"
    );
    Ok(cred.uid)
}

#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]
fn peer_uid(stream: &UnixStream) -> Result<u32> {
    let (mut user, mut group) = (0, 0);
    // getpeereid writes into the two valid uid_t/gid_t outputs.
    let rc = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut user, &mut group) };
    ensure!(rc == 0, "IPC_UNTRUSTED: cannot verify peer credentials");
    Ok(user)
}

#[cfg(all(
    unix,
    not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))
))]
fn peer_uid(_: &UnixStream) -> Result<u32> {
    anyhow::bail!("IPC_UNTRUSTED: peer credentials are unsupported on this platform")
}

#[cfg(windows)]
pub(super) fn token(path: &Path) -> Result<String> {
    let token = read_private(path, TOKEN_LEN as u64)?.context("IPC_UNTRUSTED: token is missing")?;
    ensure!(
        token.len() == TOKEN_LEN && token.bytes().all(|b| b.is_ascii_hexdigit()),
        "IPC_UNTRUSTED: token must contain 64 hex digits"
    );
    Ok(token)
}

/// A small regular file in the private folder, read without following links; None when missing.
#[cfg(windows)]
pub(super) fn read_private(path: &Path, max: u64) -> Result<Option<String>> {
    let file = match OpenOptions::new().read(true).custom_flags(FILE_FLAG_OPEN_REPARSE_POINT).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("IPC_UNTRUSTED: opening private file without following links"),
    };
    let meta = file.metadata().context("IPC_UNTRUSTED: checking private file")?;
    ensure!(
        meta.is_file() && meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0 && meta.len() <= max,
        "IPC_UNTRUSTED: {} must be a small file, not a link",
        path.display()
    );
    let mut text = String::new();
    file.take(max).read_to_string(&mut text).context("IPC_UNTRUSTED: reading private file")?;
    Ok(Some(text))
}

/// A new file takes the private folder's access list; `create_new` also refuses a link in its place.
#[cfg(windows)]
pub(super) fn create_token(path: &Path) -> Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path).context("IPC_UNTRUSTED: creating exclusive token file")
}

/// The peer process's user must be this one, as its uid on Unix.
#[cfg(windows)]
pub(super) fn peer(stream: &UnixStream) -> Result<()> {
    let (mut pid, mut length) = (0u32, 0u32);
    // The ioctl writes the peer's process id into the one u32 it is given. It reports no length, so a pid of 0
    // means it wrote nothing.
    let asked = unsafe {
        WSAIoctl(
            stream.as_raw_socket() as SOCKET,
            SIO_AF_UNIX_GETPEERPID,
            null(),
            0,
            (&mut pid as *mut u32).cast(),
            4,
            &mut length,
            null_mut(),
            None,
        )
    };
    ensure!(asked == 0 && pid != 0, "IPC_UNTRUSTED: cannot verify peer credentials");
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    ensure!(!process.is_null(), "IPC_UNTRUSTED: cannot verify peer credentials");
    // OpenProcess returned a handle of its own.
    let process = unsafe { OwnedHandle::from_raw_handle(process) };
    verify_user(&process_user(process.as_raw_handle())?)
}

#[cfg(windows)]
fn verify_user(peer: &str) -> Result<()> {
    ensure!(peer == user()?, "IPC_UNTRUSTED: peer belongs to a different user");
    Ok(())
}

pub(super) fn equal_token(left: &str, right: &str) -> bool {
    if left.len() != TOKEN_LEN || right.len() != TOKEN_LEN {
        return false;
    }
    let mut difference = 0u8;
    for (a, b) in left.bytes().zip(right.bytes()) {
        difference |= std::hint::black_box(a ^ b);
    }
    std::hint::black_box(difference) == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[cfg(windows)]
    #[test]
    fn reject_linked_token_and_shared_or_foreign_directory() {
        use std::io::Write;
        use std::os::windows::fs::{symlink_dir, symlink_file};
        let dir = std::env::temp_dir().join(format!("ipc-trust-{}", nuzky_engine::edit::new_id()));
        create_directory(&dir).unwrap();
        directory(&dir).unwrap();
        let secret = dir.join("secret");
        fs::write(&secret, "a".repeat(TOKEN_LEN)).unwrap();
        let path = dir.join("token");
        symlink_file(&secret, &path).unwrap();
        assert!(token(&path).unwrap_err().to_string().starts_with("IPC_UNTRUSTED"));
        assert!(create_token(&path).is_err());
        fs::remove_file(&path).unwrap();
        create_token(&path).unwrap().write_all("a".repeat(TOKEN_LEN).as_bytes()).unwrap();
        assert_eq!(token(&path).unwrap().len(), TOKEN_LEN);
        for text in ["a".repeat(TOKEN_LEN - 1), "g".repeat(TOKEN_LEN), "a".repeat(TOKEN_LEN + 1)] {
            fs::write(&path, text).unwrap();
            assert!(token(&path).is_err());
        }
        // A plain folder takes the temp folder's access list, which also lets in SYSTEM and administrators.
        let shared = dir.with_extension("shared");
        fs::create_dir(&shared).unwrap();
        assert!(directory(&shared).is_err());
        fs::remove_dir(shared).unwrap();
        let link = dir.with_extension("link");
        symlink_dir(&dir, &link).unwrap();
        assert!(directory(&link).is_err());
        fs::remove_dir(link).unwrap();
        assert!(directory(Path::new(&std::env::var("SystemRoot").unwrap())).is_err());
        assert!(verify_user("S-1-5-18").is_err());
        let (a, _) = UnixStream::pair().unwrap();
        peer(&a).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn reject_symlink_token_wrong_modes_and_foreign_directory() {
        let dir = std::env::temp_dir().join(format!("ipc-trust-{}", nuzky_engine::edit::new_id()));
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        directory(&dir).unwrap();
        let secret = dir.join("secret");
        fs::write(&secret, "PRIVATE KEY MUST NEVER BE SENT").unwrap();
        let path = dir.join("token");
        symlink(&secret, &path).unwrap();
        assert!(token(&path).unwrap_err().to_string().starts_with("IPC_UNTRUSTED"));
        assert!(create_token(&path).is_err());
        fs::remove_file(&path).unwrap();
        fs::write(&path, "a".repeat(TOKEN_LEN)).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(token(&path).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(token(&path).unwrap().len(), TOKEN_LEN);
        for text in ["a".repeat(TOKEN_LEN - 1), "g".repeat(TOKEN_LEN), "a".repeat(TOKEN_LEN + 1)] {
            fs::write(&path, text).unwrap();
            assert!(token(&path).is_err());
        }
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(directory(&dir).is_err());
        let link = dir.with_extension("link");
        symlink(&dir, &link).unwrap();
        assert!(directory(&link).is_err());
        fs::remove_file(link).unwrap();
        // / is foreign-owned for unprivileged test runs. Root can test a foreign uid directly.
        if uid() != 0 {
            assert!(directory(Path::new("/")).is_err());
        }
        assert!(verify_uid(uid().wrapping_add(1)).is_err());
        let (a, _) = UnixStream::pair().unwrap();
        peer(&a).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn token_comparison_checks_every_position_and_length() {
        let token = "a".repeat(TOKEN_LEN);
        assert!(equal_token(&token, &token));
        for i in 0..TOKEN_LEN {
            let mut other = token.clone().into_bytes();
            other[i] = b'b';
            assert!(!equal_token(&token, std::str::from_utf8(&other).unwrap()));
        }
        assert!(!equal_token(&token, &token[..TOKEN_LEN - 1]));
        assert!(!equal_token("", ""));
    }
}

use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

const FILE_LIMIT: u64 = 1024 * 1024;
const FILES: [&str; 5] = [
    "diagnostics.jsonl",
    "diagnostics.1.jsonl",
    "diagnostics.2.jsonl",
    "diagnostics.3.jsonl",
    "diagnostics.4.jsonl",
];

pub(super) struct LogFiles {
    current: Option<File>,
    length: u64,
    directory: Directory,
    _lock: File,
}

impl LogFiles {
    pub(super) fn open(data_dir: &Path) -> io::Result<Self> {
        let directory = Directory::open(data_dir)?;
        let lock = directory.file(".diagnostics.lock", true)?;
        lock.try_lock()?;
        let current = directory.file(FILES[0], true)?;
        let length = repair(&current)?;
        let mut next_archive = 1;
        for (age, name) in FILES.iter().enumerate().skip(1) {
            match directory.file(name, false) {
                Ok(archive) => {
                    repair(&archive)?;
                    drop(archive);
                    // Fill gaps left by a crash before the next rotation drops
                    // the oldest archive.
                    if age != next_archive {
                        directory.rename(name, FILES[next_archive])?;
                    }
                    next_archive += 1;
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(Self {
            current: Some(current),
            length,
            directory,
            _lock: lock,
        })
    }

    pub(super) fn append(&mut self, line: &[u8]) -> io::Result<()> {
        if self.length + line.len() as u64 > FILE_LIMIT {
            self.rotate()?;
        }
        let file = self.current.as_mut().expect("open current diagnostic log");
        file.seek(SeekFrom::Start(self.length))?;
        file.write_all(line)?;
        self.length += line.len() as u64;
        Ok(())
    }

    fn rotate(&mut self) -> io::Result<()> {
        // Close the file for Windows before moving names. Descending renames
        // leave any records that survive an interrupted rotation in time order.
        self.current.take();
        optional(self.directory.remove(FILES[4]))?;
        for (source, target) in FILES.iter().zip(FILES[1..].iter()).rev() {
            optional(self.directory.rename(source, target))?;
        }
        self.current = Some(self.directory.file(FILES[0], true)?);
        self.length = 0;
        Ok(())
    }
}

fn optional(result: io::Result<()>) -> io::Result<()> {
    match result {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

fn repair(mut file: &File) -> io::Result<u64> {
    let length = file.metadata()?.len();
    let mut end = length.min(FILE_LIMIT);
    let mut buffer = [0; 4096];
    while end > 0 {
        let read = end.min(buffer.len() as u64) as usize;
        let start = end - read as u64;
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut buffer[..read])?;
        if let Some(last_newline) = buffer[..read].iter().rposition(|byte| *byte == b'\n') {
            end = start + last_newline as u64 + 1;
            break;
        }
        end = start;
    }
    if end < length {
        file.set_len(end)?;
    }
    Ok(end)
}

fn unsafe_path() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "unsafe diagnostic log path",
    )
}

#[cfg(unix)]
struct Directory(File);

#[cfg(unix)]
impl Directory {
    fn open(data_dir: &Path) -> io::Result<Self> {
        use rustix::fs::{Mode, OFlags, fchmod, mkdirat, open, openat};

        fs::create_dir_all(data_dir)?;
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let profile = open(data_dir, flags, Mode::empty())?;
        match mkdirat(&profile, "logs", Mode::RWXU) {
            Ok(()) | Err(rustix::io::Errno::EXIST) => {}
            Err(error) => return Err(error.into()),
        }
        let logs = openat(&profile, "logs", flags, Mode::empty())?;
        fchmod(&logs, Mode::RWXU)?;
        Ok(Self(File::from(logs)))
    }

    fn file(&self, name: &str, create: bool) -> io::Result<File> {
        use rustix::fs::{Mode, OFlags, fchmod, openat};
        use std::os::unix::fs::MetadataExt;

        let mut flags = OFlags::RDWR | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
        flags.set(OFlags::CREATE, create);
        let mode = Mode::RUSR | Mode::WUSR;
        let file = File::from(openat(&self.0, name, flags, mode)?);
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.nlink() != 1 {
            return Err(unsafe_path());
        }
        fchmod(&file, mode)?;
        Ok(file)
    }

    fn rename(&self, source: &str, target: &str) -> io::Result<()> {
        Ok(rustix::fs::renameat(&self.0, source, &self.0, target)?)
    }

    fn remove(&self, name: &str) -> io::Result<()> {
        Ok(rustix::fs::unlinkat(
            &self.0,
            name,
            rustix::fs::AtFlags::empty(),
        )?)
    }
}

#[cfg(windows)]
struct Directory {
    path: std::path::PathBuf,
    _logs: File,
    _profile: File,
}

#[cfg(windows)]
impl Directory {
    fn open(data_dir: &Path) -> io::Result<Self> {
        fs::create_dir_all(data_dir)?;
        let profile = Self::directory(data_dir)?;
        let path = data_dir.join("logs");
        match fs::create_dir(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        let logs = Self::directory(&path)?;
        Ok(Self {
            path,
            _logs: logs,
            _profile: profile,
        })
    }

    fn directory(path: &Path) -> io::Result<File> {
        use std::os::windows::fs::OpenOptionsExt;

        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        const FILE_SHARE_READ_WRITE: u32 = 3;
        let file = File::options()
            .read(true)
            .share_mode(FILE_SHARE_READ_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;
        if !file.metadata()?.is_dir() || is_reparse(&file)? {
            return Err(unsafe_path());
        }
        Ok(file)
    }

    fn file(&self, name: &str, create: bool) -> io::Result<File> {
        use std::os::windows::fs::OpenOptionsExt;

        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        const FILE_SHARE_READ_WRITE: u32 = 3;
        let file = File::options()
            .read(true)
            .write(true)
            .create(create)
            .share_mode(FILE_SHARE_READ_WRITE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(self.path.join(name))?;
        if !file.metadata()?.is_file()
            || is_reparse(&file)?
            || winapi_util::file::information(&file)?.number_of_links() != 1
        {
            return Err(unsafe_path());
        }
        Ok(file)
    }

    fn rename(&self, source: &str, target: &str) -> io::Result<()> {
        fs::rename(self.path.join(source), self.path.join(target))
    }

    fn remove(&self, name: &str) -> io::Result<()> {
        fs::remove_file(self.path.join(name))
    }
}

#[cfg(windows)]
fn is_reparse(file: &File) -> io::Result<bool> {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    Ok(file.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
}

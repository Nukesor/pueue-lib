use std::collections::VecDeque;
use std::fs::{read_dir, remove_file, File};
use std::io::{self, Cursor, Read, Seek};
use std::path::{Path, PathBuf};

use log::error;
use snap::write::FrameEncoder;

use crate::error::Error;

/// Return the paths to the `(stdout, stderr)` log files of a task.
pub fn get_log_paths(task_id: usize, path: &Path) -> (PathBuf, PathBuf) {
    let task_log_dir = path.join("task_logs");
    let out_path = task_log_dir.join(format!("{}_stdout.log", task_id));
    let err_path = task_log_dir.join(format!("{}_stderr.log", task_id));
    (out_path, err_path)
}

/// Create and return the file handle for the `(stdout, stderr)` log files of a task.
pub fn create_log_file_handles(task_id: usize, path: &Path) -> Result<(File, File), Error> {
    let (out_path, err_path) = get_log_paths(task_id, path);
    let stdout = File::create(out_path)?;
    let stderr = File::create(err_path)?;

    Ok((stdout, stderr))
}

/// Return the file handle for the `(stdout, stderr)` log files of a task.
pub fn get_log_file_handles(task_id: usize, path: &Path) -> Result<(File, File), Error> {
    let (out_path, err_path) = get_log_paths(task_id, path);
    let stdout = File::open(out_path)?;
    let stderr = File::open(err_path)?;

    Ok((stdout, stderr))
}

/// Remove the the log files of a task.
pub fn clean_log_handles(task_id: usize, path: &Path) {
    let (out_path, err_path) = get_log_paths(task_id, path);
    if out_path.exists() {
        if let Err(err) = remove_file(out_path) {
            error!(
                "Failed to remove stdout file for task {} with error {:?}",
                task_id, err
            );
        };
    }
    if err_path.exists() {
        if let Err(err) = remove_file(err_path) {
            error!(
                "Failed to remove stderr file for task {} with error {:?}",
                task_id, err
            );
        };
    }
}

/// Return the `(stdout, stderr)` output of a task. \
/// Task output is compressed using [snap] to save some memory and bandwidth.
pub fn read_and_compress_log_files(
    task_id: usize,
    path: &Path,
    lines: Option<usize>,
) -> Result<(Vec<u8>, Vec<u8>), Error> {
    let (mut stdout_file, mut stderr_file) = get_log_file_handles(task_id, path)?;

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    if let Some(lines) = lines {
        // Get the last few lines of both files
        let stdout_bytes = read_last_lines(&mut stdout_file, lines).into_bytes();
        let stderr_bytes = read_last_lines(&mut stderr_file, lines).into_bytes();
        let mut stdout_cursor = Cursor::new(stdout_bytes);
        let mut stderr_cursor = Cursor::new(stderr_bytes);

        // Compress the partial log input and pipe it into the snappy compressor
        let mut stdout_compressor = FrameEncoder::new(&mut stdout);
        io::copy(&mut stdout_cursor, &mut stdout_compressor)?;
        let mut stderr_compressor = FrameEncoder::new(&mut stderr);
        io::copy(&mut stderr_cursor, &mut stderr_compressor)?;
    } else {
        // Compress the full log input and pipe it into the snappy compressor
        let mut stdout_compressor = FrameEncoder::new(&mut stdout);
        io::copy(&mut stdout_file, &mut stdout_compressor)?;
        let mut stderr_compressor = FrameEncoder::new(&mut stderr);
        io::copy(&mut stderr_file, &mut stderr_compressor)?;
    }

    Ok((stdout, stderr))
}

/// Return the last lines of `(stdout, stderr)` of a task. \
/// This output is uncompressed and may take a lot of memory, which is why we only read
/// the last few lines.
pub fn read_last_log_file_lines(
    task_id: usize,
    path: &Path,
    lines: usize,
) -> Result<(String, String), Error> {
    let (mut stdout_file, mut stderr_file) = match get_log_file_handles(task_id, path) {
        Ok((stdout, stderr)) => (stdout, stderr),
        Err(err) => {
            return Err(Error::LogRead(format!(
                "Error while opening log files for task {}: {}",
                task_id, err
            )));
        }
    };

    // Get the last few lines of both files
    Ok((
        read_last_lines(&mut stdout_file, lines),
        read_last_lines(&mut stderr_file, lines),
    ))
}

/// Remove all files in the log directory.
pub fn reset_task_log_directory(path: &Path) -> Result<(), Error> {
    let task_log_dir = path.join("task_logs");

    let files = read_dir(task_log_dir)?;

    for file in files.flatten() {
        if let Err(err) = remove_file(file.path()) {
            error!("Failed to delete log file: {}", err);
        }
    }

    Ok(())
}

pub fn read_last_lines_as_byte_deque(file: &mut File, lines: usize) -> Result<VecDeque<u8>, Error> {
    let chunk_size = 4096u32;
    let mut buf = vec![0u8; chunk_size as usize];
    let size = match file.seek(io::SeekFrom::End(0)) {
        Ok(p) => p,
        Err(err) => {
            return Err(Error::LogSeek(format!(
                "Error while seeking log file : {}",
                err
            )));
        }
    };
    let mut position = size;
    let mut text = std::collections::VecDeque::new();
    let mut lines_seen = 0usize;
    'outer: loop {
        if position == 0 {
            break;
        }
        let next_chunk_size = if position > chunk_size as u64 {
            chunk_size
        } else {
            position as u32
        };
        position = match file.seek(io::SeekFrom::Current(-(next_chunk_size as i64))) {
            Ok(p) => p,
            Err(err) => {
                return Err(Error::LogSeek(format!(
                    "Error while seeking log file : {}",
                    err
                )));
            }
        };
        let slice = &mut buf[..next_chunk_size as usize];
        if let Err(err) = file.read_exact(slice) {
            return Err(Error::LogRead(format!(
                "Error while reading log file : {}",
                err
            )));
        }
        for b in slice.iter().rev() {
            if *b == b'\n' {
                lines_seen += 1;
                if lines_seen == lines + 1 {
                    break 'outer;
                }
            }
            text.push_front(*b)
        }
    }
    if let Err(err) = file.seek(io::SeekFrom::End(0)) {
        return Err(Error::LogSeek(format!(
            "Error while seeking log file : {}",
            err
        )));
    };
    Ok(text)
}

/// Read the last `amount` lines of a file to a string.
pub fn read_last_lines(file: &mut File, amount: usize) -> String {
    match read_last_lines_as_byte_deque(file, amount) {
        Ok(deque) => {
            let lines: Vec<u8> = deque.into();
            match String::from_utf8(lines) {
                Ok(lines) => lines,
                Err(error) => {
                    return format!("(Pueue error) Failed to read last lines of file: {}", error)
                }
            }
        }
        Err(error) => return format!("(Pueue error) Failed to read last lines of file: {}", error),
    }
}

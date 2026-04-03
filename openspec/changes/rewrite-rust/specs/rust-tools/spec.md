## ADDED Requirements

### Requirement: read_file Tool
The `read_file` tool SHALL read a file from disk and return its contents with 1-based line numbers prefixed to each line. It MUST support `offset` (first line to return, 1-based) and `limit` (maximum lines to return) parameters to enable partial reads of large files.

#### Scenario: Reading a full file
- **WHEN** `read_file` is called with only `path`
- **THEN** the full file contents are returned with every line prefixed as `<n>\t<content>`

#### Scenario: Reading a slice
- **WHEN** `read_file` is called with `offset: 10, limit: 20`
- **THEN** exactly lines 10–29 are returned, prefixed with their original line numbers

#### Scenario: File not found
- **WHEN** `read_file` is called with a path that does not exist
- **THEN** the tool result has `is_error: true` and the content describes the missing file

---

### Requirement: write_file Tool
The `write_file` tool SHALL create or overwrite a file at the specified path with the provided text content. Parent directories MUST be created automatically if they do not exist.

#### Scenario: Creating a new file
- **WHEN** `write_file` is called with a path in a non-existent directory
- **THEN** all intermediate directories are created and the file is written successfully

#### Scenario: Overwriting an existing file
- **WHEN** `write_file` is called on an existing path
- **THEN** the old content is replaced atomically and the tool result confirms the byte count written

---

### Requirement: edit_file Tool
The `edit_file` tool SHALL perform a surgical in-place replacement: it MUST locate the first occurrence of `old_string` in the file and replace it with `new_string`. If `old_string` is not found the tool MUST fail with `is_error: true`.

#### Scenario: Successful replacement
- **WHEN** `edit_file` is called with an `old_string` that appears exactly once in the file
- **THEN** only that occurrence is replaced and all other file content is preserved

#### Scenario: Ambiguous match
- **WHEN** `old_string` appears more than once and `replace_all` is false
- **THEN** the tool fails with `is_error: true` and a message indicating the occurrence count

---

### Requirement: bash Tool
The `bash` tool SHALL execute a shell command using `/bin/sh -c` and return combined stdout and stderr. It MUST honour a configurable `timeout_ms` parameter (default 120 000 ms) and a cancellation signal; on timeout or cancellation the child process MUST be killed and the tool result MUST have `is_error: true`.

#### Scenario: Successful command
- **WHEN** `bash` is called with `command: "echo hello"`
- **THEN** the tool result content contains `"hello\n"` and `is_error` is false

#### Scenario: Command times out
- **WHEN** `bash` is called with a command that does not terminate within `timeout_ms`
- **THEN** the process is killed and the tool result has `is_error: true` with a timeout message

---

### Requirement: grep Tool
The `grep` tool SHALL search file contents using a regular expression and return matching lines with their file path and 1-based line number. It MUST support a `context` parameter (lines before and after each match) and a `glob` parameter to restrict which files are searched.

#### Scenario: Basic regex search
- **WHEN** `grep` is called with `pattern: "fn main"` and `path: "src/"`
- **THEN** every line in `src/` matching the pattern is returned as `<file>:<line>:<content>`

#### Scenario: Context lines
- **WHEN** `grep` is called with `context: 2`
- **THEN** each match is accompanied by up to 2 lines before and after, separated by `--` between match groups

---

### Requirement: find Tool
The `find` tool SHALL perform a glob-pattern file search rooted at a given directory and return matching paths sorted by modification time, newest first. It MUST support a `head_limit` parameter to cap the number of results.

#### Scenario: Glob search
- **WHEN** `find` is called with `pattern: "**/*.rs"` and `path: "crates/"`
- **THEN** all `.rs` files under `crates/` are returned as absolute paths

#### Scenario: Result cap
- **WHEN** `head_limit: 10` is set and more than 10 files match
- **THEN** only the 10 most recently modified paths are returned

---

### Requirement: ls Tool
The `ls` tool SHALL list the immediate entries of a directory, returning each entry's name, type (`file` or `dir`), size in bytes, and last-modified timestamp. It MUST NOT recurse into subdirectories.

#### Scenario: Listing a directory
- **WHEN** `ls` is called with a valid directory path
- **THEN** each entry is returned with its name, type, size, and ISO-8601 modified timestamp

#### Scenario: Path is a file
- **WHEN** `ls` is called with a path pointing to a file rather than a directory
- **THEN** the tool fails with `is_error: true` and a message indicating the path is not a directory

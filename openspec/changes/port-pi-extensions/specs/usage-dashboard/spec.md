## ADDED Requirements

### Requirement: usage-read-sessions
`pi usage` SHALL read all session files from `~/.pi/sessions/` and parse them to extract usage data.

#### Scenario: Sessions directory exists with files
- **WHEN** the user runs `pi usage` and `~/.pi/sessions/` contains one or more session JSON files
- **THEN** the system reads every file in the directory and proceeds to extract usage metrics from each

#### Scenario: Sessions directory is empty or absent
- **WHEN** the user runs `pi usage` and `~/.pi/sessions/` does not exist or contains no files
- **THEN** the system prints a message indicating no session data is available and exits without error

---

### Requirement: usage-parse-fields
The parser SHALL extract the following fields from assistant messages in each session file: `input` tokens, `output` tokens, `cache_read` tokens, `cache_write` tokens, and `cost.total`.

#### Scenario: Assistant message with complete usage fields
- **WHEN** a session file contains an assistant message with a usage object including all five fields
- **THEN** all five values are extracted and accumulated for that session's statistics

#### Scenario: Assistant message with partial usage fields
- **WHEN** a session file contains an assistant message where some usage fields are absent or null
- **THEN** missing fields default to zero and the remaining fields are still accumulated correctly

---

### Requirement: usage-deduplication
The system SHALL deduplicate entries by computing a hash of each entry's timestamp combined with its total token count, discarding any entry whose hash has already been seen.

#### Scenario: Duplicate entries across session files
- **WHEN** the same assistant message appears in more than one session file (same timestamp and total tokens)
- **THEN** only one copy is counted in the aggregated statistics

---

### Requirement: usage-grouping
The system SHALL group aggregated statistics by provider and then by model, computing totals for each group per period (today, this week, all time).

#### Scenario: Multiple providers and models present
- **WHEN** session data includes messages from multiple providers and models
- **THEN** each unique provider/model combination appears as its own row in the output, with accurate totals for sessions, messages, cost, input tokens, and output tokens

---

### Requirement: usage-table-display
The system SHALL display a formatted table with the columns: Provider/Model, Sessions, Messages, Cost, Input Tokens, Output Tokens.

#### Scenario: Displaying the usage table
- **WHEN** `pi usage` completes parsing and grouping
- **THEN** a table is printed to stdout with one row per provider/model group, aligned columns, and a summary totals row at the bottom

---

### Requirement: usage-period-flag
`pi usage --period <value>` SHALL filter statistics to the selected period. Valid values are `today`, `week`, and `all` (default: `all`).

#### Scenario: Filtering to today
- **WHEN** the user runs `pi usage --period today`
- **THEN** only messages with a timestamp falling on the current calendar day are included in the statistics

#### Scenario: Filtering to this week
- **WHEN** the user runs `pi usage --period week`
- **THEN** only messages with a timestamp within the current ISO week are included in the statistics

#### Scenario: Default period
- **WHEN** the user runs `pi usage` without specifying `--period`
- **THEN** all available session data is included regardless of date

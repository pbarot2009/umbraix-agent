CREATE TABLE IF NOT EXISTS user_keys (
    user_id      INTEGER PRIMARY KEY,
    ciphertext   BLOB    NOT NULL,
    nonce        BLOB    NOT NULL,
    key_hint     TEXT    NOT NULL,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    last_used_at INTEGER
);

CREATE TABLE IF NOT EXISTS guild_keys (
    guild_id     INTEGER PRIMARY KEY,
    ciphertext   BLOB    NOT NULL,
    nonce        BLOB    NOT NULL,
    key_hint     TEXT    NOT NULL,
    set_by       INTEGER NOT NULL,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    last_used_at INTEGER
);

CREATE TABLE IF NOT EXISTS guild_config (
    guild_id        INTEGER PRIMARY KEY,
    enabled         INTEGER NOT NULL DEFAULT 1,
    allow_user_byok INTEGER NOT NULL DEFAULT 1,
    model           TEXT,
    cooldown_secs   INTEGER,
    log_channel_id  INTEGER,
    allow_role_id   INTEGER,
    updated_at      INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS usage_daily (
    day          INTEGER NOT NULL,
    subject_kind TEXT    NOT NULL,
    subject_id   INTEGER NOT NULL,
    requests     INTEGER NOT NULL DEFAULT 0,
    tool_calls   INTEGER NOT NULL DEFAULT 0,
    errors       INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (day, subject_kind, subject_id)
);

CREATE TABLE IF NOT EXISTS audit_log (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    ts         INTEGER NOT NULL,
    guild_id   INTEGER,
    user_id    INTEGER NOT NULL,
    key_source TEXT,
    action     TEXT    NOT NULL,
    detail     TEXT,
    success    INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_audit_guild_ts ON audit_log (guild_id, ts);

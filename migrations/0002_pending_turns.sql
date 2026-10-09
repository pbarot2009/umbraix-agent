CREATE TABLE IF NOT EXISTS pending_turns (
    request_id      TEXT    PRIMARY KEY,
    guild_id        INTEGER NOT NULL,
    channel_id      INTEGER NOT NULL,
    user_id         INTEGER NOT NULL,
    user_name       TEXT    NOT NULL DEFAULT '',
    prompt          TEXT    NOT NULL,
    history_json    TEXT    NOT NULL DEFAULT '[]',
    tool_summary    TEXT    NOT NULL DEFAULT '',
    status          TEXT    NOT NULL DEFAULT 'pending',
    attempts        INTEGER NOT NULL DEFAULT 0,
    max_attempts    INTEGER NOT NULL DEFAULT 2,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    last_error      TEXT
);

CREATE INDEX IF NOT EXISTS idx_pending_turns_status_updated
    ON pending_turns (status, updated_at);
CREATE INDEX IF NOT EXISTS idx_pending_turns_channel_user
    ON pending_turns (channel_id, user_id);

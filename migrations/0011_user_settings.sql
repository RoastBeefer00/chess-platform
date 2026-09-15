-- Free-form user preferences (auto-queen, future board/piece themes, etc).
-- JSONB rather than typed columns so new settings don't need a migration —
-- `UserSettings` fields are all `#[serde(default)]`, so a key missing from
-- an old row (or simply absent from '{}') just falls back to its default.
ALTER TABLE users ADD COLUMN settings JSONB NOT NULL DEFAULT '{}'::jsonb;

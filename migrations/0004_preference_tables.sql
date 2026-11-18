CREATE TABLE IF NOT EXISTS user_settings (
    user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    compactness SMALLINT NOT NULL DEFAULT 0,
    theme TEXT NOT NULL DEFAULT 'light'
);

CREATE TABLE IF NOT EXISTS board_subscriptions (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    board_slug TEXT NOT NULL,
    PRIMARY KEY (user_id, board_slug)
);

CREATE TABLE IF NOT EXISTS hidden_threads (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    thread_id UUID NOT NULL,
    PRIMARY KEY (user_id, thread_id)
);

CREATE TABLE IF NOT EXISTS saved_threads (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    thread_id UUID NOT NULL,
    PRIMARY KEY (user_id, thread_id)
);

CREATE TABLE IF NOT EXISTS votes (
    post_id UUID NOT NULL,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    value SMALLINT NOT NULL CHECK (value BETWEEN -1 AND 1),
    PRIMARY KEY (post_id, user_id)
);

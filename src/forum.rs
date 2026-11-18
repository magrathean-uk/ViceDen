//! In-memory forum store used by the web UI.
//!
//! The storage is intentionally lightweight and local for this stage. It gives
//! the UI immediate "fully functioning" behavior for posting and browsing
//! while preserving a migration path to SQL-backed storage in the future.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tokio::sync::Mutex;
use tokio::{runtime::Handle, task::block_in_place};
use tracing::warn;
use uuid::Uuid;

use crate::auth::{self, hash_password, legacy_password_hash, verify_password};
use crate::models::{Post, PostState, Thread, UserRole};

pub const ROOT_USER_ID: Uuid = Uuid::from_u128(42);
pub const ROOT_HANDLE: &str = "root";
const DEFAULT_STATE_FILE: &str = "data/forum_state.json";
const MIN_COMPACTNESS: i8 = -2;
const MAX_COMPACTNESS: i8 = 2;
const THREAD_SCORE_HIDE_HOURS: i64 = 3;
const MAX_MENTIONS_PER_POST: usize = 10;
const THREAD_POSTS_KEEP_LARGE_THREAD: usize = 3;
const LOGIN_THROTTLE_WINDOW_MINUTES: i64 = 15;
const MAX_FAILED_LOGINS_PER_WINDOW: usize = 5;
const REGISTRATION_THROTTLE_WINDOW_MINUTES: i64 = 30;
const MAX_REGISTRATIONS_PER_WINDOW: usize = 3;
const MAX_THREAD_TAGS: usize = 3;
const MAX_THREAD_TAG_CHARS: usize = 24;

mod notifications;
mod preferences;
mod relationships;
mod reports;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForumStorageMode {
    LocalJson,
    OptionalPostgres,
    RequiredPostgres,
}

impl ForumStorageMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::LocalJson => "local_json",
            Self::OptionalPostgres => "optional_postgres",
            Self::RequiredPostgres => "required_postgres",
        }
    }
}

fn clamp_compactness(value: i8) -> i8 {
    value.clamp(MIN_COMPACTNESS, MAX_COMPACTNESS)
}

fn make_session_token() -> String {
    auth::tokens::new_session_token()
}

fn now() -> DateTime<Utc> {
    Utc::now()
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum NotificationKind {
    Mention,
    Reply,
    PostRemoved,
    Report,
    ModAction,
    Invite,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ForumNotification {
    pub id: Uuid,
    pub actor_id: Option<Uuid>,
    pub kind: NotificationKind,
    pub thread_id: Option<Uuid>,
    pub thread_title: String,
    pub post_id: Option<Uuid>,
    pub message: String,
    pub created_at: DateTime<Utc>,
    pub is_read: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct UserProfileSettings {
    pub compactness: i8,
    pub theme: String,
    #[serde(default = "default_notifications_enabled")]
    pub notify_mentions: bool,
    #[serde(default = "default_notifications_enabled")]
    pub notify_replies: bool,
    #[serde(default = "default_notifications_enabled")]
    pub notify_moderation: bool,
}

impl Default for UserProfileSettings {
    fn default() -> Self {
        Self {
            compactness: 0,
            theme: "dark".to_string(),
            notify_mentions: true,
            notify_replies: true,
            notify_moderation: true,
        }
    }
}

fn default_notifications_enabled() -> bool {
    true
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ForumUserRecord {
    pub id: Uuid,
    pub handle: String,
    pub password_hash: String,
    #[serde(default = "default_user_role")]
    pub role: UserRole,
    #[serde(default)]
    pub is_banned: bool,
    #[serde(default)]
    pub probation_until: Option<DateTime<Utc>>,
    #[serde(default)]
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub last_known_ip: Option<String>,
}

fn default_user_role() -> UserRole {
    UserRole::Member
}

impl ForumUserRecord {
    pub fn is_founder(&self) -> bool {
        self.role == UserRole::Founder
    }

    pub fn is_member(&self) -> bool {
        self.role == UserRole::Member
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub user_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub csrf_token: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ApiTokenRecord {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    #[serde(default)]
    pub preview: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

#[derive(Clone)]
pub struct ApiTokenSummary {
    pub id: Uuid,
    pub name: String,
    pub preview: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

#[derive(Clone)]
pub struct IssuedApiToken {
    pub id: Uuid,
    pub name: String,
    pub token: String,
    pub preview: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ReportRecord {
    pub id: Uuid,
    pub actor_id: Uuid,
    pub target_post_id: Option<Uuid>,
    pub target_thread_id: Option<Uuid>,
    pub reason: String,
    pub details: Option<String>,
    pub created_at: DateTime<Utc>,
    pub resolved: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ModerationEvent {
    pub id: Uuid,
    pub actor_id: Uuid,
    pub target_post_id: Option<Uuid>,
    pub target_thread_id: Option<Uuid>,
    pub reason: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum PersistedUser {
    Legacy(String),
    Full(ForumUserRecord),
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum PersistedApiToken {
    Legacy(Uuid),
    Full(ApiTokenRecord),
}

#[derive(Serialize, Deserialize)]
struct PersistedForumStore {
    #[serde(default)]
    boards_ttl: HashMap<String, u32>,
    #[serde(default)]
    threads_by_board: HashMap<String, Vec<Uuid>>,
    #[serde(default)]
    threads: HashMap<Uuid, Thread>,
    #[serde(default)]
    posts: HashMap<Uuid, Vec<Post>>,
    #[serde(default)]
    users: HashMap<Uuid, PersistedUser>,
    #[serde(default)]
    notifications: HashMap<Uuid, Vec<ForumNotification>>,
    #[serde(default)]
    user_settings: HashMap<Uuid, UserProfileSettings>,
    #[serde(default)]
    board_subscriptions: HashMap<Uuid, HashSet<String>>,
    #[serde(default)]
    hidden_threads: HashMap<Uuid, HashSet<Uuid>>,
    #[serde(default)]
    saved_threads: HashMap<Uuid, HashSet<Uuid>>,
    #[serde(default)]
    votes: HashMap<Uuid, HashMap<Uuid, i8>>,
    #[serde(default)]
    sessions: HashMap<String, SessionRecord>,
    #[serde(default)]
    api_tokens: HashMap<String, PersistedApiToken>,
    #[serde(default)]
    reports: Vec<ReportRecord>,
    #[serde(default)]
    moderation_events: Vec<ModerationEvent>,
    #[serde(default)]
    thread_tags: HashMap<Uuid, Vec<String>>,
    #[serde(default)]
    thread_states: HashMap<Uuid, ThreadState>,
    #[serde(default)]
    post_edits: HashMap<Uuid, Vec<PostEditRecord>>,
    #[serde(default)]
    muted_users: HashMap<Uuid, HashSet<Uuid>>,
    #[serde(default)]
    tag_subscriptions: HashMap<Uuid, HashSet<String>>,
    #[serde(default)]
    invites: HashMap<String, InviteRecord>,
    #[serde(default)]
    banned_ips: HashSet<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ThreadState {
    pub pinned: bool,
    pub locked: bool,
    pub answered: bool,
    pub closed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PostEditRecord {
    pub post_id: Uuid,
    pub editor_id: Uuid,
    pub previous_body: String,
    pub new_body: String,
    pub edited_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InviteRecord {
    pub code: String,
    pub label: String,
    pub inviter_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub redeemed_by: Option<Uuid>,
    pub redeemed_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug)]
pub struct AdminMetrics {
    pub total_users: usize,
    pub total_threads: usize,
    pub total_posts: usize,
    pub open_reports: usize,
    pub unresolved_notifications: usize,
}

pub struct ForumStore {
    boards_ttl: HashMap<String, u32>,
    threads_by_board: HashMap<String, Vec<Uuid>>,
    threads: HashMap<Uuid, Thread>,
    posts: HashMap<Uuid, Vec<Post>>,
    users: HashMap<Uuid, ForumUserRecord>,
    notifications: HashMap<Uuid, Vec<ForumNotification>>,
    user_settings: HashMap<Uuid, UserProfileSettings>,
    board_subscriptions: HashMap<Uuid, HashSet<String>>,
    hidden_threads: HashMap<Uuid, HashSet<Uuid>>,
    saved_threads: HashMap<Uuid, HashSet<Uuid>>,
    votes: HashMap<Uuid, HashMap<Uuid, i8>>,
    sessions: HashMap<String, SessionRecord>,
    api_tokens: HashMap<String, ApiTokenRecord>,
    reports: Vec<ReportRecord>,
    moderation_events: Vec<ModerationEvent>,
    thread_tags: HashMap<Uuid, Vec<String>>,
    thread_states: HashMap<Uuid, ThreadState>,
    post_edits: HashMap<Uuid, Vec<PostEditRecord>>,
    muted_users: HashMap<Uuid, HashSet<Uuid>>,
    tag_subscriptions: HashMap<Uuid, HashSet<String>>,
    invites: HashMap<String, InviteRecord>,
    board_downvote_enabled: HashMap<String, bool>,
    banned_ips: HashSet<String>,
    failed_login_attempts: HashMap<String, Vec<DateTime<Utc>>>,
    registration_attempts: HashMap<String, Vec<DateTime<Utc>>>,
    state_path: PathBuf,
    token_pepper: Option<String>,
    db_pool: Option<PgPool>,
    storage_mode: ForumStorageMode,
    last_storage_error: Option<String>,
    /// True when both Postgres and the moderation pipeline are wired up.
    /// New posts/threads fail closed (`mod_state='pending'`) so a content
    /// classifier outage cannot publish unmoderated content. Toggled by
    /// `app::build_router` / `set_moderation_enabled` at boot.
    moderation_enabled: bool,
}

pub type SharedForumStore = Arc<Mutex<ForumStore>>;

impl Default for ForumStore {
    fn default() -> Self {
        Self {
            boards_ttl: HashMap::new(),
            threads_by_board: HashMap::new(),
            threads: HashMap::new(),
            posts: HashMap::new(),
            users: HashMap::new(),
            notifications: HashMap::new(),
            user_settings: HashMap::new(),
            board_subscriptions: HashMap::new(),
            hidden_threads: HashMap::new(),
            saved_threads: HashMap::new(),
            votes: HashMap::new(),
            sessions: HashMap::new(),
            api_tokens: HashMap::new(),
            reports: Vec::new(),
            moderation_events: Vec::new(),
            thread_tags: HashMap::new(),
            thread_states: HashMap::new(),
            post_edits: HashMap::new(),
            muted_users: HashMap::new(),
            tag_subscriptions: HashMap::new(),
            invites: HashMap::new(),
            board_downvote_enabled: HashMap::new(),
            banned_ips: HashSet::new(),
            failed_login_attempts: HashMap::new(),
            registration_attempts: HashMap::new(),
            state_path: PathBuf::from(DEFAULT_STATE_FILE),
            token_pepper: None,
            db_pool: None,
            storage_mode: ForumStorageMode::LocalJson,
            last_storage_error: None,
            moderation_enabled: false,
        }
    }
}

impl ForumStore {
    pub fn seeded(boards: &[crate::models::Board]) -> Self {
        Self::with_state_path(boards, DEFAULT_STATE_FILE)
    }

    pub fn with_state_path(boards: &[crate::models::Board], state_path: impl AsRef<Path>) -> Self {
        let state_path = state_path.as_ref().to_path_buf();
        let mut store = Self::load_from_disk_or_seed(boards, state_path.as_path());
        store.state_path = state_path;
        store.persist_state();
        store
    }

    pub async fn with_state_path_and_pool(
        boards: &[crate::models::Board],
        state_path: impl AsRef<Path>,
        db_pool: Option<PgPool>,
    ) -> Self {
        let state_path = state_path.as_ref().to_path_buf();
        let mut store = if let Some(pool) = db_pool.clone() {
            Self::load_from_postgres_or_seed(boards, state_path.as_path(), &pool).await
        } else {
            Self::load_from_disk_or_seed(boards, state_path.as_path())
        };
        store.state_path = state_path;
        store.storage_mode = if db_pool.is_some() {
            ForumStorageMode::OptionalPostgres
        } else {
            ForumStorageMode::LocalJson
        };
        store.db_pool = db_pool;
        store.persist_state();
        store
    }

    pub async fn try_with_state_path_and_pool(
        boards: &[crate::models::Board],
        state_path: impl AsRef<Path>,
        db_pool: Option<PgPool>,
    ) -> Result<Self, anyhow::Error> {
        let state_path = state_path.as_ref().to_path_buf();
        let mut store = if let Some(pool) = db_pool.clone() {
            Self::load_from_postgres_or_seed(boards, state_path.as_path(), &pool).await
        } else {
            Self::try_load_from_disk_or_seed(boards, state_path.as_path())?
        };
        store.state_path = state_path;
        store.storage_mode = if db_pool.is_some() {
            ForumStorageMode::OptionalPostgres
        } else {
            ForumStorageMode::LocalJson
        };
        store.db_pool = db_pool;
        store.try_persist_state()?;
        Ok(store)
    }

    pub fn set_required_postgres(&mut self, required: bool) {
        self.storage_mode = if required {
            ForumStorageMode::RequiredPostgres
        } else if self.db_pool.is_some() {
            ForumStorageMode::OptionalPostgres
        } else {
            ForumStorageMode::LocalJson
        };
    }

    pub fn set_token_pepper(&mut self, token_pepper: Option<String>) {
        self.token_pepper =
            token_pepper.and_then(|value| (!value.trim().is_empty()).then_some(value));
    }

    /// Toggle the fail-closed insert mode. When `true`, `create_thread_*`
    /// and `create_post_*` ask the SQL layer to land new rows with
    /// `mod_state='pending'` so the moderation pipeline must explicitly flip
    /// them to `'visible'` before they show up. Set at boot from
    /// `state.moderation.is_some() && state.db.is_some()`.
    pub fn set_moderation_enabled(&mut self, enabled: bool) {
        self.moderation_enabled = enabled;
    }

    pub fn postgres_required(&self) -> bool {
        self.requires_postgres()
    }

    #[cfg(test)]
    fn set_required_postgres_for_tests(&mut self, pool: PgPool) {
        self.db_pool = Some(pool);
        self.storage_mode = ForumStorageMode::RequiredPostgres;
        self.last_storage_error = None;
    }

    pub fn storage_mode_label(&self) -> &'static str {
        self.storage_mode.label()
    }

    pub fn database_status_label(&self) -> &'static str {
        if self.storage_mode == ForumStorageMode::LocalJson {
            "not_configured"
        } else if self.db_pool.is_none() {
            "missing"
        } else if self.last_storage_error.is_some() {
            "error"
        } else {
            "ok"
        }
    }

    fn requires_postgres(&self) -> bool {
        self.storage_mode == ForumStorageMode::RequiredPostgres
    }

    fn token_lookup_digests(&self, raw_token: &str) -> Vec<String> {
        auth::tokens::lookup_digests(raw_token, self.token_pepper.as_deref())
    }

    fn token_storage_digest(&self, raw_token: &str) -> auth::tokens::TokenDigest {
        auth::tokens::digest_token(raw_token, self.token_pepper.as_deref())
    }

    fn remove_session_by_raw_token(&mut self, raw_token: &str) -> bool {
        let digests = self.token_lookup_digests(raw_token);
        let mut removed = false;
        for digest in digests {
            removed |= self.sessions.remove(&digest).is_some();
        }
        removed
    }

    fn remove_api_token_by_raw_token(&mut self, raw_token: &str) -> bool {
        let digests = self.token_lookup_digests(raw_token);
        let mut removed = false;
        for digest in digests {
            removed |= self.api_tokens.remove(&digest).is_some();
        }
        removed
    }

    fn storage_error(&mut self, message: impl Into<String>) -> ForumActionError {
        let message = message.into();
        self.last_storage_error = Some(message.clone());
        ForumActionError::Storage(message)
    }

    fn record_storage_failure(&mut self, action: &str, error: &dyn std::fmt::Display) {
        self.last_storage_error = Some(format!("{action}: {error}"));
    }

    /// Builds the demo seed data. Seeding goes through the mutating methods, which persist as they
    /// go, so the store must already point at the caller's `state_path`; otherwise those writes land
    /// in the default `data/forum_state.json` relative to the working directory.
    fn seed_from_boards(boards: &[crate::models::Board], state_path: &Path) -> Self {
        let mut store = Self { state_path: state_path.to_path_buf(), ..Self::default() };
        let root = ForumUserRecord {
            id: ROOT_USER_ID,
            handle: ROOT_HANDLE.to_string(),
            password_hash: hash_password(ROOT_HANDLE),
            role: UserRole::Founder,
            is_banned: false,
            probation_until: None,
            created_at: now(),
            last_known_ip: None,
        };

        store.users.insert(root.id, root);

        store.users.insert(
            Uuid::from_u128(1001),
            ForumUserRecord {
                id: Uuid::from_u128(1001),
                handle: "atlas".to_string(),
                password_hash: hash_password("atlas"),
                role: UserRole::Member,
                is_banned: false,
                probation_until: None,
                created_at: now() - Duration::hours(8),
                last_known_ip: None,
            },
        );
        store.users.insert(
            Uuid::from_u128(1002),
            ForumUserRecord {
                id: Uuid::from_u128(1002),
                handle: "luna".to_string(),
                password_hash: hash_password("luna"),
                role: UserRole::Member,
                is_banned: false,
                probation_until: None,
                created_at: now() - Duration::hours(8),
                last_known_ip: None,
            },
        );
        store.users.insert(
            Uuid::from_u128(1003),
            ForumUserRecord {
                id: Uuid::from_u128(1003),
                handle: "jade".to_string(),
                password_hash: hash_password("jade"),
                role: UserRole::Member,
                is_banned: false,
                probation_until: None,
                created_at: now() - Duration::hours(8),
                last_known_ip: None,
            },
        );
        store.users.insert(
            Uuid::from_u128(1004),
            ForumUserRecord {
                id: Uuid::from_u128(1004),
                handle: "viper".to_string(),
                password_hash: hash_password("viper"),
                role: UserRole::Member,
                is_banned: false,
                probation_until: None,
                created_at: now() - Duration::hours(8),
                last_known_ip: None,
            },
        );
        store.users.insert(
            Uuid::from_u128(1005),
            ForumUserRecord {
                id: Uuid::from_u128(1005),
                handle: "modbot".to_string(),
                password_hash: hash_password("modbot"),
                role: UserRole::Mod,
                is_banned: false,
                probation_until: None,
                created_at: now() - Duration::hours(8),
                last_known_ip: None,
            },
        );

        let now = now();

        for (board_index, board) in boards.iter().enumerate() {
            store.boards_ttl.insert(board.slug.clone(), board.ttl_days);
            store.board_downvote_enabled.insert(board.slug.clone(), true);

            for idx in 0..2 {
                let is_root = idx == 0;
                let author_id = if is_root {
                    ROOT_USER_ID
                } else {
                    let ids = [
                        Uuid::from_u128(1001),
                        Uuid::from_u128(1002),
                        Uuid::from_u128(1003),
                        Uuid::from_u128(1004),
                    ];
                    ids[(board_index + idx) % ids.len()]
                };

                let title = if is_root {
                    format!("{} rules and expectations", board.name)
                } else {
                    format!("{} open discussion", board.name)
                };

                let body = if is_root {
                    format!(
                        "Start thread in {} with safe, clear discussion. Keep it anonymised and useful.",
                        board.name
                    )
                } else {
                    format!(
                        "Shared experience from {}: what worked, what failed, what to do next.",
                        board.name
                    )
                };
                let body_for_post = body.clone();

                let thread = Thread {
                    id: Uuid::new_v4(),
                    board_slug: board.slug.clone(),
                    title: title.clone(),
                    user_id: author_id,
                    created_at: now - Duration::hours(((board_index * 10) + idx * 4) as i64),
                    expires_at: now + Duration::days(180)
                        - Duration::hours(((board_index * 4) + idx) as i64),
                };
                let thread_id = thread.id;
                let first_post = self_post(
                    author_id,
                    body_for_post,
                    thread_id,
                    thread.created_at + Duration::minutes(2),
                );

                store.threads_by_board.entry(board.slug.clone()).or_default().push(thread_id);
                store.threads.insert(thread_id, thread.clone());

                let first_reply = Post {
                    id: Uuid::new_v4(),
                    thread_id,
                    user_id: if is_root { Uuid::from_u128(1001) } else { ROOT_USER_ID },
                    body: if is_root {
                        "Welcome. Stay calm and keep replies constructive.".to_string()
                    } else {
                        format!("Interesting opener for {}.", board.name)
                    },
                    created_at: thread.created_at + Duration::hours(6),
                    state: PostState::Allow,
                };
                let second_reply = Post {
                    id: Uuid::new_v4(),
                    thread_id,
                    user_id: if is_root { ROOT_USER_ID } else { Uuid::from_u128(1002) },
                    body: format!(
                        "Good point on {}. I agree with that approach.",
                        board.name.to_lowercase()
                    ),
                    created_at: thread.created_at + Duration::hours(8),
                    state: PostState::Allow,
                };

                let seed_reply = if is_root {
                    "@jade check this, this fits what you posted in other threads.".to_string()
                } else {
                    "@luna you should see this thread too".to_string()
                };

                let mut seed_posts = vec![first_post, first_reply, second_reply];
                seed_posts.push(self_post(
                    Uuid::from_u128(1003),
                    seed_reply,
                    thread_id,
                    thread.created_at + Duration::hours(10),
                ));
                store.posts.insert(thread_id, seed_posts);

                let removed_actor = Uuid::from_u128(1005);
                let removed_post = Uuid::new_v4();
                store.create_mod_event_removed(
                    thread_id,
                    Some(removed_post),
                    removed_actor,
                    "seed cleanup".to_string(),
                    "System import sample".to_string(),
                );
                store.add_notification(
                    Uuid::from_u128(1001),
                    Some(removed_actor),
                    NotificationKind::PostRemoved,
                    Some(thread_id),
                    Some(removed_post),
                    Some(format!("Your post in '{}' was removed after review.", title)),
                );

                store.notify_mentions_for_content(thread_id, author_id, &body);
            }
        }

        for ids in store.threads_by_board.values_mut() {
            ids.sort_by_key(|id| store.threads[id].created_at);
            ids.reverse();
        }

        for board in boards {
            let Some(first_thread_id) =
                store.threads_by_board.get(&board.slug).and_then(|ids| ids.first()).copied()
            else {
                continue;
            };
            let thread_title =
                store.thread_title(&first_thread_id).unwrap_or_else(|| board.name.clone());
            store.add_notification(
                ROOT_USER_ID,
                Some(Uuid::from_u128(1002)),
                NotificationKind::Mention,
                Some(first_thread_id),
                None,
                Some(format!("You were mentioned in '{thread_title}' by luna.")),
            );
            store.add_notification(
                ROOT_USER_ID,
                None,
                NotificationKind::PostRemoved,
                Some(first_thread_id),
                None,
                Some(format!("A post in '{thread_title}' was removed for duplicate content.")),
            );
        }

        let sample_root_thread = store.threads.values().next().map(|first| first.id);
        if let Some(thread_id) = sample_root_thread {
            store.add_notification(
                ROOT_USER_ID,
                Some(Uuid::from_u128(1001)),
                NotificationKind::Mention,
                Some(thread_id),
                None,
                Some("@atlas mentioned you in the root thread".to_string()),
            );
        }

        store
    }

    fn load_from_disk_or_seed(boards: &[crate::models::Board], state_path: &Path) -> Self {
        let mut store = Self { state_path: state_path.to_path_buf(), ..Self::default() };

        match fs::read_to_string(state_path) {
            Ok(raw) => match serde_json::from_str::<PersistedForumStore>(&raw) {
                Ok(persisted) => store.apply_persisted(persisted),
                Err(error) => warn!(
                    path = %state_path.display(),
                    error = %error,
                    "state file is invalid; reseeding local forum state"
                ),
            },
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => warn!(
                path = %state_path.display(),
                error = %error,
                "state file could not be read; reseeding local forum state"
            ),
        }

        if store.users.is_empty() {
            warn!(path = %state_path.display(), "local forum state missing; seeding fresh data");
            store = Self::seed_from_boards(boards, state_path);
        }

        store.ensure_root_presence();
        store.ensure_defaults(boards);
        store.restore_root_password_if_missing();
        store
    }

    fn try_load_from_disk_or_seed(
        boards: &[crate::models::Board],
        state_path: &Path,
    ) -> Result<Self, anyhow::Error> {
        let mut store = Self { state_path: state_path.to_path_buf(), ..Self::default() };
        let mut loaded_existing = false;

        match fs::read_to_string(state_path) {
            Ok(raw) => {
                loaded_existing = true;
                let persisted =
                    serde_json::from_str::<PersistedForumStore>(&raw).map_err(|error| {
                        anyhow::anyhow!("state file is invalid: {}: {error}", state_path.display())
                    })?;
                store.apply_persisted(persisted);
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                anyhow::bail!("state file could not be read: {}: {error}", state_path.display());
            }
        }

        if loaded_existing && store.users.is_empty() {
            anyhow::bail!("state file is invalid: {}: no users", state_path.display());
        }

        if !loaded_existing {
            store = Self::seed_from_boards(boards, state_path);
        }

        store.ensure_root_presence();
        store.ensure_defaults(boards);
        store.restore_root_password_if_missing();
        Ok(store)
    }

    async fn load_from_postgres_or_seed(
        boards: &[crate::models::Board],
        state_path: &Path,
        pool: &PgPool,
    ) -> Self {
        let mut store = Self { state_path: state_path.to_path_buf(), ..Self::default() };

        match crate::db::load_normalized_forum_data(pool).await {
            Ok(normalized) if !normalized.users.is_empty() => {
                store.apply_normalized(normalized, boards);
            }
            Ok(_) => {
                warn!("postgres normalized tables are empty; falling back to runtime snapshot");
            }
            Err(error) => {
                warn!(
                    error = %error,
                    "postgres normalized tables could not be read; falling back to runtime snapshot"
                );
            }
        }

        if store.users.is_empty() {
            match crate::db::load_forum_runtime_state(pool).await {
                Ok(Some(raw)) => match serde_json::from_str::<PersistedForumStore>(&raw) {
                    Ok(persisted) => store.apply_persisted(persisted),
                    Err(error) => {
                        warn!(
                            error = %error,
                            "postgres forum runtime state is invalid; seeding fresh data"
                        );
                    }
                },
                Ok(None) => warn!("postgres forum runtime state is empty; seeding fresh data"),
                Err(error) => {
                    warn!(
                        error = %error,
                        "postgres forum runtime state could not be read; seeding fresh data"
                    );
                }
            }
        }

        if store.users.is_empty() {
            store = Self::seed_from_boards(boards, state_path);
        }

        store.ensure_root_presence();
        store.ensure_defaults(boards);
        store.restore_root_password_if_missing();
        store
    }

    fn apply_normalized(
        &mut self,
        normalized: crate::db::NormalizedForumData,
        boards: &[crate::models::Board],
    ) {
        self.boards_ttl = boards.iter().map(|board| (board.slug.clone(), board.ttl_days)).collect();
        self.threads = normalized.threads;
        self.posts = normalized.posts;
        self.users = normalized.users;
        self.notifications = normalized.notifications;
        self.user_settings = normalized.user_settings;
        self.board_subscriptions = normalized.board_subscriptions;
        self.hidden_threads = normalized.hidden_threads;
        self.saved_threads = normalized.saved_threads;
        self.votes = normalized.votes;
        self.reports = normalized.reports;
        self.moderation_events = normalized.moderation_events;
        self.thread_tags = normalized.thread_tags;
        self.thread_states = normalized.thread_states;
        self.post_edits = normalized.post_edits;
        self.muted_users = normalized.muted_users;
        self.tag_subscriptions = normalized.tag_subscriptions;
        self.invites = normalized.invites;
        self.banned_ips = normalized.banned_ips;
        self.threads_by_board.clear();
        for thread in self.threads.values() {
            self.threads_by_board.entry(thread.board_slug.clone()).or_default().push(thread.id);
        }
        for thread_ids in self.threads_by_board.values_mut() {
            thread_ids.sort_by_key(|id| self.threads[id].created_at);
            thread_ids.reverse();
        }
    }

    fn apply_persisted(&mut self, persisted: PersistedForumStore) {
        self.boards_ttl = persisted.boards_ttl;
        self.threads_by_board = persisted.threads_by_board;
        self.threads = persisted.threads;
        self.posts = persisted.posts;
        self.notifications = persisted.notifications;
        self.user_settings = persisted.user_settings;
        self.board_subscriptions = persisted.board_subscriptions;
        self.hidden_threads = persisted.hidden_threads;
        self.saved_threads = persisted.saved_threads;
        self.votes = persisted.votes;
        self.sessions = persisted
            .sessions
            .into_iter()
            .map(|(key, value)| {
                let digest = if key.starts_with(auth::tokens::HMAC_SHA256_PREFIX)
                    || key.starts_with(auth::tokens::LEGACY_SHA256_PREFIX)
                {
                    key
                } else {
                    auth::tokens::legacy_digest_for_lookup(&key)
                };
                (digest, value)
            })
            .collect();
        self.api_tokens = persisted
            .api_tokens
            .into_iter()
            .map(|(key, value)| {
                let record = match value {
                    PersistedApiToken::Legacy(user_id) => ApiTokenRecord {
                        id: Uuid::new_v4(),
                        user_id,
                        name: "Legacy token".to_string(),
                        preview: "legacy...".to_string(),
                        created_at: now() - Duration::hours(24),
                        last_used_at: None,
                    },
                    PersistedApiToken::Full(mut record) => {
                        if record.preview.trim().is_empty() {
                            record.preview = "token...".to_string();
                        }
                        record
                    }
                };
                let digest = if key.starts_with(auth::tokens::HMAC_SHA256_PREFIX)
                    || key.starts_with(auth::tokens::LEGACY_SHA256_PREFIX)
                {
                    key
                } else {
                    auth::tokens::legacy_digest_for_lookup(&key)
                };
                (digest, record)
            })
            .collect();
        self.reports = persisted.reports;
        self.moderation_events = persisted.moderation_events;
        self.thread_tags = persisted.thread_tags;
        self.thread_states = persisted.thread_states;
        self.post_edits = persisted.post_edits;
        self.muted_users = persisted.muted_users;
        self.tag_subscriptions = persisted.tag_subscriptions;
        self.invites = persisted.invites;
        self.banned_ips = persisted.banned_ips;

        for (id, user_record) in persisted.users {
            let user = match user_record {
                PersistedUser::Legacy(name) => ForumUserRecord {
                    id,
                    handle: name,
                    password_hash: legacy_password_hash(&id.to_string(), &id.to_string()),
                    role: if id == ROOT_USER_ID { UserRole::Founder } else { UserRole::Member },
                    is_banned: false,
                    probation_until: None,
                    created_at: now() - Duration::hours(24),
                    last_known_ip: None,
                },
                PersistedUser::Full(account) => account,
            };
            self.users.insert(id, user);
        }
    }

    fn ensure_root_presence(&mut self) {
        self.users.entry(ROOT_USER_ID).or_insert_with(|| ForumUserRecord {
            id: ROOT_USER_ID,
            handle: ROOT_HANDLE.to_string(),
            password_hash: hash_password(ROOT_HANDLE),
            role: UserRole::Founder,
            is_banned: false,
            probation_until: None,
            created_at: now() - Duration::hours(48),
            last_known_ip: None,
        });
    }

    fn restore_root_password_if_missing(&mut self) {
        if let Some(root) = self.users.get_mut(&ROOT_USER_ID) {
            if root.password_hash.trim().is_empty() {
                root.password_hash = hash_password(ROOT_HANDLE);
            }
            root.role = UserRole::Founder;
        }
        let _ = self.users.get(&ROOT_USER_ID);
    }

    fn ensure_defaults(&mut self, boards: &[crate::models::Board]) {
        self.user_settings.entry(ROOT_USER_ID).or_default();
        if self.notifications.is_empty() {
            self.notifications.insert(ROOT_USER_ID, Vec::new());
        }

        self.board_subscriptions
            .entry(ROOT_USER_ID)
            .or_insert_with(|| default_subscription_set(boards));
        self.hidden_threads.entry(ROOT_USER_ID).or_default();
        self.saved_threads.entry(ROOT_USER_ID).or_default();

        for board in boards {
            self.boards_ttl.entry(board.slug.clone()).or_insert(board.ttl_days);
            self.threads_by_board.entry(board.slug.clone()).or_default();
            self.board_downvote_enabled.entry(board.slug.clone()).or_insert(true);
        }

        for user_id in self.users.keys() {
            self.user_settings.entry(*user_id).or_default();
            self.board_subscriptions.entry(*user_id).or_default();
            self.hidden_threads.entry(*user_id).or_default();
            self.saved_threads.entry(*user_id).or_default();
        }

        let valid_board_slugs: HashSet<String> = self.boards_ttl.keys().cloned().collect();
        let valid_thread_ids: HashSet<Uuid> = self.threads.keys().copied().collect();

        for subscriptions in self.board_subscriptions.values_mut() {
            subscriptions.retain(|slug| valid_board_slugs.contains(slug));
        }
        for hidden in self.hidden_threads.values_mut() {
            hidden.retain(|thread_id| valid_thread_ids.contains(thread_id));
        }
        for saved in self.saved_threads.values_mut() {
            saved.retain(|thread_id| valid_thread_ids.contains(thread_id));
        }
        self.thread_tags.retain(|thread_id, tags| {
            tags.retain(|tag| !tag.trim().is_empty());
            valid_thread_ids.contains(thread_id)
        });
        self.thread_states.retain(|thread_id, _| valid_thread_ids.contains(thread_id));
        self.post_edits.retain(|post_id, _| {
            self.posts.values().any(|posts| posts.iter().any(|post| post.id == *post_id))
        });
        self.muted_users.retain(|user_id, muted| {
            muted.retain(|target_id| self.users.contains_key(target_id));
            self.users.contains_key(user_id)
        });
        self.tag_subscriptions.retain(|user_id, tags| {
            tags.retain(|tag| !tag.trim().is_empty());
            self.users.contains_key(user_id)
        });

        self.remove_expired_sessions();
    }

    pub fn persist_state(&self) {
        self.persist_state_with_sync(true);
    }

    fn sync_admin_user_mutation_to_postgres(&self) -> bool {
        let Some(pool) = self.db_pool.clone() else {
            return false;
        };

        let users = self.users.clone();
        let user_settings = self.user_settings.clone();
        let board_subscriptions = self.board_subscriptions.clone();
        let hidden_threads = self.hidden_threads.clone();
        let saved_threads = self.saved_threads.clone();
        let votes = self.votes.clone();
        let notifications = self.notifications.clone();
        let reports = self.reports.clone();
        let moderation_events = self.moderation_events.clone();
        let banned_ips = self.banned_ips.clone();
        let threads = self.threads.clone();
        let posts = self.posts.clone();
        let thread_tags = self.thread_tags.clone();
        let thread_states = self.thread_states.clone();
        let post_edits = self.post_edits.clone();
        let muted_users = self.muted_users.clone();
        let tag_subscriptions = self.tag_subscriptions.clone();
        let invites = self.invites.clone();

        block_in_place(|| {
            Handle::current()
                .block_on(async move {
                    crate::db::sync_content_tables(&pool, &threads, &posts).await?;
                    crate::db::sync_thread_tags(&pool, &thread_tags).await?;
                    crate::db::sync_next_feature_tables(
                        &pool,
                        &thread_states,
                        &post_edits,
                        &muted_users,
                        &tag_subscriptions,
                        &invites,
                    )
                    .await?;
                    crate::db::sync_preference_tables(
                        &pool,
                        &user_settings,
                        &board_subscriptions,
                        &hidden_threads,
                        &saved_threads,
                        &votes,
                    )
                    .await?;
                    crate::db::sync_moderation_tables(
                        &pool,
                        &notifications,
                        &reports,
                        &moderation_events,
                        &banned_ips,
                    )
                    .await?;
                    crate::db::sync_users(&pool, &users).await
                })
                .map_err(|error| {
                    warn!(
                        error = %error,
                        "admin user mutation sync in postgres failed; falling back to full sync"
                    );
                    error
                })
                .is_ok()
        })
    }

    fn persist_runtime_state(&self) {
        self.persist_state_with_sync(false);
    }

    fn try_persist_state(&self) -> Result<(), anyhow::Error> {
        let json = serde_json::to_string_pretty(&self.persisted_snapshot()).map_err(|error| {
            anyhow::anyhow!("state serialization failed: {}: {error}", self.state_path.display())
        })?;

        if let Some(pool) = self.db_pool.as_ref() {
            let payload = json.clone();
            let runtime_pool = pool.clone();
            block_in_place(|| {
                Handle::current()
                    .block_on(async move {
                        crate::db::save_forum_runtime_state(&runtime_pool, &payload).await
                    })
                    .map_err(|error| {
                        anyhow::anyhow!("postgres forum runtime state write failed: {error}")
                    })
            })?;
            let auth_pool = pool.clone();
            let users = self.users.clone();
            block_in_place(|| {
                Handle::current()
                    .block_on(async move { crate::db::sync_users(&auth_pool, &users).await })
                    .map_err(|error| anyhow::anyhow!("postgres user table sync failed: {error}"))
            })?;
            let pref_pool = pool.clone();
            let user_settings = self.user_settings.clone();
            let board_subscriptions = self.board_subscriptions.clone();
            let hidden_threads = self.hidden_threads.clone();
            let saved_threads = self.saved_threads.clone();
            let votes = self.votes.clone();
            block_in_place(|| {
                Handle::current()
                    .block_on(async move {
                        crate::db::sync_preference_tables(
                            &pref_pool,
                            &user_settings,
                            &board_subscriptions,
                            &hidden_threads,
                            &saved_threads,
                            &votes,
                        )
                        .await
                    })
                    .map_err(|error| {
                        anyhow::anyhow!("postgres preference table sync failed: {error}")
                    })
            })?;
            let moderation_pool = pool.clone();
            let notifications = self.notifications.clone();
            let reports = self.reports.clone();
            let moderation_events = self.moderation_events.clone();
            let banned_ips = self.banned_ips.clone();
            block_in_place(|| {
                Handle::current()
                    .block_on(async move {
                        crate::db::sync_moderation_tables(
                            &moderation_pool,
                            &notifications,
                            &reports,
                            &moderation_events,
                            &banned_ips,
                        )
                        .await
                    })
                    .map_err(|error| {
                        anyhow::anyhow!("postgres moderation table sync failed: {error}")
                    })
            })?;
            let content_pool = pool.clone();
            let threads = self.threads.clone();
            let posts = self.posts.clone();
            let thread_tags = self.thread_tags.clone();
            let thread_states = self.thread_states.clone();
            let post_edits = self.post_edits.clone();
            let muted_users = self.muted_users.clone();
            let tag_subscriptions = self.tag_subscriptions.clone();
            let invites = self.invites.clone();
            block_in_place(|| {
                Handle::current()
                    .block_on(async move {
                        crate::db::sync_content_tables(&content_pool, &threads, &posts).await?;
                        crate::db::sync_thread_tags(&content_pool, &thread_tags).await?;
                        crate::db::sync_next_feature_tables(
                            &content_pool,
                            &thread_states,
                            &post_edits,
                            &muted_users,
                            &tag_subscriptions,
                            &invites,
                        )
                        .await
                    })
                    .map_err(|error| anyhow::anyhow!("postgres content table sync failed: {error}"))
            })?;
            return Ok(());
        }

        if let Some(parent) = self.state_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                anyhow::anyhow!("state directory creation failed: {}: {error}", parent.display())
            })?;
        }
        fs::write(&self.state_path, json).map_err(|error| {
            anyhow::anyhow!("state file write failed: {}: {error}", self.state_path.display())
        })?;
        Ok(())
    }

    fn persist_state_with_sync(&self, sync_tables: bool) {
        let json = match serde_json::to_string_pretty(&self.persisted_snapshot()) {
            Ok(value) => value,
            Err(error) => {
                warn!(
                    path = %self.state_path.display(),
                    error = %error,
                    "state serialization failed; local changes not written"
                );
                return;
            }
        };

        if let Some(pool) = self.db_pool.as_ref() {
            let payload = json.clone();
            let runtime_pool = pool.clone();
            block_in_place(|| {
                if let Err(error) = Handle::current().block_on(async move {
                    crate::db::save_forum_runtime_state(&runtime_pool, &payload).await
                }) {
                    warn!(error = %error, "postgres forum runtime state write failed");
                }
            });
            if !sync_tables {
                return;
            }
            let auth_pool = pool.clone();
            let users = self.users.clone();
            block_in_place(|| {
                if let Err(error) = Handle::current()
                    .block_on(async move { crate::db::sync_users(&auth_pool, &users).await })
                {
                    warn!(error = %error, "postgres user table sync failed");
                }
            });
            let pref_pool = pool.clone();
            let user_settings = self.user_settings.clone();
            let board_subscriptions = self.board_subscriptions.clone();
            let hidden_threads = self.hidden_threads.clone();
            let saved_threads = self.saved_threads.clone();
            let votes = self.votes.clone();
            block_in_place(|| {
                if let Err(error) = Handle::current().block_on(async move {
                    crate::db::sync_preference_tables(
                        &pref_pool,
                        &user_settings,
                        &board_subscriptions,
                        &hidden_threads,
                        &saved_threads,
                        &votes,
                    )
                    .await
                }) {
                    warn!(error = %error, "postgres preference table sync failed");
                }
            });
            let moderation_pool = pool.clone();
            let notifications = self.notifications.clone();
            let reports = self.reports.clone();
            let moderation_events = self.moderation_events.clone();
            let banned_ips = self.banned_ips.clone();
            block_in_place(|| {
                if let Err(error) = Handle::current().block_on(async move {
                    crate::db::sync_moderation_tables(
                        &moderation_pool,
                        &notifications,
                        &reports,
                        &moderation_events,
                        &banned_ips,
                    )
                    .await
                }) {
                    warn!(error = %error, "postgres moderation table sync failed");
                }
            });
            let content_pool = pool.clone();
            let threads = self.threads.clone();
            let posts = self.posts.clone();
            let thread_tags = self.thread_tags.clone();
            let thread_states = self.thread_states.clone();
            let post_edits = self.post_edits.clone();
            let muted_users = self.muted_users.clone();
            let tag_subscriptions = self.tag_subscriptions.clone();
            let invites = self.invites.clone();
            block_in_place(|| {
                if let Err(error) = Handle::current().block_on(async move {
                    crate::db::sync_content_tables(&content_pool, &threads, &posts).await?;
                    crate::db::sync_thread_tags(&content_pool, &thread_tags).await?;
                    crate::db::sync_next_feature_tables(
                        &content_pool,
                        &thread_states,
                        &post_edits,
                        &muted_users,
                        &tag_subscriptions,
                        &invites,
                    )
                    .await
                }) {
                    warn!(error = %error, "postgres content table sync failed");
                }
            });
            return;
        }

        if let Some(parent) = self.state_path.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                warn!(
                    path = %parent.display(),
                    error = %error,
                    "state directory creation failed; local changes not written"
                );
                return;
            }
        }
        if let Err(error) = fs::write(&self.state_path, json) {
            warn!(
                path = %self.state_path.display(),
                error = %error,
                "state file write failed; local changes not written"
            );
        }
    }

    fn persisted_snapshot(&self) -> PersistedForumStore {
        PersistedForumStore {
            boards_ttl: self.boards_ttl.clone(),
            threads_by_board: self.threads_by_board.clone(),
            threads: self.threads.clone(),
            posts: self.posts.clone(),
            users: self
                .users
                .iter()
                .map(|(id, user)| (*id, PersistedUser::Full(user.clone())))
                .collect(),
            notifications: self.notifications.clone(),
            user_settings: self.user_settings.clone(),
            board_subscriptions: self.board_subscriptions.clone(),
            hidden_threads: self.hidden_threads.clone(),
            saved_threads: self.saved_threads.clone(),
            votes: self.votes.clone(),
            sessions: self.sessions.clone(),
            api_tokens: self
                .api_tokens
                .iter()
                .map(|(token, record)| (token.clone(), PersistedApiToken::Full(record.clone())))
                .collect(),
            reports: self.reports.clone(),
            moderation_events: self.moderation_events.clone(),
            thread_tags: self.thread_tags.clone(),
            thread_states: self.thread_states.clone(),
            post_edits: self.post_edits.clone(),
            muted_users: self.muted_users.clone(),
            tag_subscriptions: self.tag_subscriptions.clone(),
            invites: self.invites.clone(),
            banned_ips: self.banned_ips.clone(),
        }
    }

    pub fn board_threads(&self, board_slug: &str) -> Vec<Thread> {
        self.all_threads().into_iter().filter(|thread| thread.board_slug == board_slug).collect()
    }

    pub fn board_threads_with_scores(
        &self,
        board_slug: &str,
        sort: Option<&str>,
    ) -> Vec<ThreadPreview> {
        let mut items: Vec<ThreadPreview> = self
            .board_threads(board_slug)
            .into_iter()
            .map(|thread| {
                let thread_id = thread.id;
                let created_at = thread.created_at;
                let posts =
                    self.thread_with_posts(&thread_id).map(|(_, posts)| posts).unwrap_or_default();
                let score = self.thread_score(thread_id);
                let activity = self.thread_last_activity(thread_id).unwrap_or(created_at);
                ThreadPreview {
                    thread,
                    score,
                    reply_count: posts.len().saturating_sub(1),
                    score_hidden: is_score_hidden(created_at),
                    last_activity: activity,
                }
            })
            .collect();

        let sort_mode = sort.unwrap_or("new").to_ascii_lowercase();
        match sort_mode.as_str() {
            "top" => {
                items.sort_by_key(|item| item.score);
                items.reverse();
            }
            "hot" => {
                items.sort_by(|a, b| {
                    hot_score(a, now())
                        .partial_cmp(&hot_score(b, now()))
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                items.reverse();
            }
            "active" => {
                items.sort_by_key(|item| item.last_activity);
                items.reverse();
            }
            "unanswered" => {
                items.retain(|item| item.reply_count == 0);
                items.sort_by_key(|item| item.thread.created_at);
                items.reverse();
            }
            _ => {
                items.sort_by_key(|item| item.thread.created_at);
                items.reverse();
            }
        }

        items
    }

    pub fn visible_board_thread_count(&self, user_id: Uuid, board_slug: &str) -> usize {
        if let Some(pool) = self.db_pool.clone() {
            let count = block_in_place(|| {
                Handle::current().block_on(async move {
                    crate::db::count_visible_board_threads(&pool, user_id, board_slug).await
                })
            });
            match count {
                Ok(count) => return count,
                Err(error) => {
                    if self.requires_postgres() {
                        return 0;
                    }
                    warn!(
                        error = %error,
                        "visible board count in postgres failed; using local cache"
                    );
                }
            }
        }
        self.board_threads_with_scores(board_slug, None)
            .into_iter()
            .filter(|item| !self.is_thread_hidden(user_id, item.thread.id))
            .count()
    }

    pub fn board_thread_page_with_scores(
        &self,
        user_id: Uuid,
        board_slug: &str,
        sort: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Vec<ThreadPreview> {
        let sort = sort.unwrap_or("new").to_ascii_lowercase();
        if let Some(pool) = self.db_pool.clone() {
            let board_slug = board_slug.to_string();
            let sort_for_db = sort.clone();
            let rows = block_in_place(|| {
                Handle::current().block_on(async move {
                    crate::db::list_board_thread_previews(
                        &pool,
                        user_id,
                        &board_slug,
                        &sort_for_db,
                        limit,
                        offset,
                    )
                    .await
                })
            });
            match rows {
                Ok(rows) => return rows,
                Err(error) => {
                    if self.requires_postgres() {
                        return Vec::new();
                    }
                    warn!(error = %error, "board page in postgres failed; using local cache");
                }
            }
        }
        self.board_threads_with_scores(board_slug, Some(&sort))
            .into_iter()
            .filter(|item| !self.is_thread_hidden(user_id, item.thread.id))
            .skip(offset)
            .take(limit)
            .collect()
    }

    pub fn preview_for_thread(&self, thread_id: Uuid) -> Option<ThreadPreview> {
        let (thread, posts) = self.thread_with_posts(&thread_id)?;
        let created_at = thread.created_at;
        let score = self.thread_score(thread_id);
        let activity = self.thread_last_activity(thread_id).unwrap_or(created_at);
        Some(ThreadPreview {
            thread,
            score,
            reply_count: posts.len().saturating_sub(1),
            score_hidden: is_score_hidden(created_at),
            last_activity: activity,
        })
    }

    pub fn board_thread_count(&self, board_slug: &str) -> usize {
        if let Some(pool) = self.db_pool.clone() {
            let count = block_in_place(|| {
                Handle::current().block_on(async move {
                    crate::db::count_threads_for_board(&pool, board_slug).await
                })
            });
            match count {
                Ok(count) => return count,
                Err(error) => {
                    if self.requires_postgres() {
                        return 0;
                    }
                    warn!(error = %error, "board thread count in postgres failed; using local cache");
                }
            }
        }

        self.threads_by_board.get(board_slug).map(|items| items.len()).unwrap_or_default()
    }

    pub fn thread_participant_count(&self, thread_id: Uuid) -> usize {
        self.users_in_thread(thread_id).len()
    }

    pub fn root_post_snippet(&self, thread_id: Uuid, limit: usize) -> String {
        self.thread_with_posts(&thread_id)
            .and_then(|(_, posts)| posts.first().cloned())
            .map(|post| truncate_plain_text(&post.body, limit))
            .unwrap_or_default()
    }

    pub fn thread_with_posts(&self, thread_id: &Uuid) -> Option<(Thread, Vec<Post>)> {
        if let Some(pool) = self.db_pool.clone() {
            let thread = block_in_place(|| {
                Handle::current().block_on(async move {
                    crate::db::load_thread_with_posts(&pool, *thread_id).await
                })
            });
            match thread {
                Ok(Some(result)) => return Some(result),
                Ok(None) => return None,
                Err(error) => {
                    if self.requires_postgres() {
                        return None;
                    }
                    warn!(error = %error, "thread load in postgres failed; using local cache");
                }
            }
        }

        self.threads
            .get(thread_id)
            .map(|thread| (thread.clone(), self.posts.get(thread_id).cloned().unwrap_or_default()))
    }

    pub fn thread_title(&self, thread_id: &Uuid) -> Option<String> {
        if let Some(pool) = self.db_pool.clone() {
            let title = block_in_place(|| {
                Handle::current()
                    .block_on(async move { crate::db::load_thread_title(&pool, *thread_id).await })
            });
            match title {
                Ok(title) => return title,
                Err(error) => {
                    if self.requires_postgres() {
                        return None;
                    }
                    warn!(error = %error, "thread title in postgres failed; using local cache");
                }
            }
        }

        self.threads.get(thread_id).map(|thread| thread.title.clone())
    }

    pub fn latest_activity_for_user(
        &self,
        user_id: Uuid,
        _boards: &[crate::models::Board],
        limit: usize,
    ) -> Vec<UserActivityTeaser> {
        let mut activity = Vec::new();

        for thread in self.all_threads() {
            if let Some((_, posts)) = self.thread_with_posts(&thread.id) {
                for post in posts {
                    if post.user_id == user_id {
                        activity.push(UserActivityTeaser {
                            board_slug: thread.board_slug.clone(),
                            thread_id: thread.id,
                            post_id: post.id,
                            thread_title: thread.title.clone(),
                            body_snippet: post.body.chars().take(90).collect::<String>(),
                            created_at: post.created_at,
                        });
                    }
                }
            }
        }

        activity.sort_by_key(|item| item.created_at);
        activity.reverse();
        activity.truncate(limit);
        activity
    }

    pub fn participants_for_thread(&self, thread_id: &Uuid) -> Vec<String> {
        let mut users = Vec::new();
        let mut seen = HashSet::new();
        let Some((thread, posts)) = self.thread_with_posts(thread_id) else {
            return users;
        };

        let add_user = |user_id: &Uuid, list: &mut Vec<String>, seen: &mut HashSet<Uuid>| {
            if seen.insert(*user_id) {
                if let Some(name) = self.user_display_name(*user_id).into() {
                    list.push(name);
                }
            }
        };

        add_user(&thread.user_id, &mut users, &mut seen);
        for post in posts {
            add_user(&post.user_id, &mut users, &mut seen);
        }

        users
    }

    pub fn create_thread(
        &mut self,
        board_slug: &str,
        title: String,
        body: String,
        user_id: Uuid,
    ) -> Option<Thread> {
        self.create_thread_with_checks(board_slug, title, body, user_id).ok()
    }

    pub fn create_thread_with_checks(
        &mut self,
        board_slug: &str,
        title: String,
        body: String,
        user_id: Uuid,
    ) -> Result<Thread, ForumActionError> {
        self.create_thread_with_tags(board_slug, title, body, user_id, Vec::new())
    }

    pub fn create_thread_with_tags(
        &mut self,
        board_slug: &str,
        title: String,
        body: String,
        user_id: Uuid,
        tags: Vec<String>,
    ) -> Result<Thread, ForumActionError> {
        if !self.can_user_post_in_board(user_id, board_slug) {
            return Err(ForumActionError::Auth("user cannot post in this board".to_string()));
        }
        let trimmed_title = title.trim();
        let trimmed_body = body.trim();
        if trimmed_title.is_empty() || trimmed_body.is_empty() {
            return Err(ForumActionError::Validation("title and body are required".to_string()));
        }
        if trimmed_title.chars().count() > 64 {
            return Err(ForumActionError::Validation(
                "title cannot exceed 64 characters".to_string(),
            ));
        }
        if trimmed_body.chars().count() > 32_000 {
            return Err(ForumActionError::Validation(
                "body cannot exceed 32,000 characters".to_string(),
            ));
        }
        reject_spam_body(trimmed_body)?;

        let normalized_title = normalize_title(trimmed_title);
        if self.threads.values().any(|thread| {
            thread.board_slug == board_slug
                && thread.expires_at > now()
                && normalize_title(&thread.title) == normalized_title
        }) {
            return Err(ForumActionError::Validation(
                "similar thread already exists in this board".to_string(),
            ));
        }

        self.check_limits(user_id, board_slug, true)?;

        let thread_id = Uuid::new_v4();
        let now = now();
        let ttl_days = match self.boards_ttl(board_slug) {
            Some(value) => value,
            None => return Err(ForumActionError::NotFound("board not found".to_string())),
        };

        let thread = Thread {
            id: thread_id,
            board_slug: board_slug.to_string(),
            title: trimmed_title.to_string(),
            user_id,
            created_at: now,
            expires_at: now + Duration::days(i64::from(ttl_days)),
        };

        let post = Post {
            id: Uuid::new_v4(),
            thread_id,
            user_id,
            body: trimmed_body.to_string(),
            created_at: now,
            state: PostState::Allow,
        };

        let normalized_tags = normalize_thread_tags(tags);

        let mod_pending = self.moderation_enabled;
        let content_synced = if let Some(pool) = self.db_pool.clone() {
            let thread_for_db = thread.clone();
            let post_for_db = post.clone();
            let tags_for_db = normalized_tags.clone();
            let inserted = block_in_place(|| {
                Handle::current().block_on(async move {
                    crate::db::insert_thread_with_post(
                        &pool,
                        &thread_for_db,
                        &post_for_db,
                        mod_pending,
                    )
                    .await?;
                    crate::db::set_thread_tags(&pool, thread_for_db.id, &tags_for_db).await
                })
            });
            match inserted {
                Ok(()) => true,
                Err(error) => {
                    if self.requires_postgres() {
                        return Err(self
                            .storage_error(format!("thread insert in postgres failed: {error}")));
                    }
                    warn!(
                        error = %error,
                        "thread insert in postgres failed; falling back to full sync"
                    );
                    false
                }
            }
        } else {
            false
        };

        self.threads_by_board.entry(board_slug.to_string()).or_default().insert(0, thread_id);
        self.threads.insert(thread_id, thread.clone());
        self.posts.insert(thread_id, vec![post.clone()]);
        if !normalized_tags.is_empty() {
            self.thread_tags.insert(thread_id, normalized_tags.clone());
        }

        self.notify_mentions_for_content(thread_id, user_id, trimmed_body);
        self.notify_tag_followers_for_thread(thread_id, user_id, &normalized_tags);
        self.add_notification(
            user_id,
            Some(user_id),
            NotificationKind::Reply,
            Some(thread_id),
            None,
            Some("Thread created.".to_string()),
        );

        if content_synced {
            self.persist_runtime_state();
        } else {
            self.persist_state();
        }
        Ok(thread)
    }

    pub fn create_post(&mut self, thread_id: Uuid, body: String, user_id: Uuid) -> Option<Post> {
        self.create_post_with_checks(thread_id, body, user_id).ok()
    }

    pub fn create_post_with_checks(
        &mut self,
        thread_id: Uuid,
        body: String,
        user_id: Uuid,
    ) -> Result<Post, ForumActionError> {
        if !self.has_thread(&thread_id) {
            return Err(ForumActionError::NotFound("thread not found".to_string()));
        }

        let trimmed_body = body.trim();
        if trimmed_body.is_empty() {
            return Err(ForumActionError::Validation("body is required".to_string()));
        }
        if trimmed_body.chars().count() > 32_000 {
            return Err(ForumActionError::Validation(
                "body cannot exceed 32,000 characters".to_string(),
            ));
        }
        reject_spam_body(trimmed_body)?;

        let board_slug = self
            .thread_board_slug(thread_id)
            .ok_or_else(|| ForumActionError::NotFound("thread not found".to_string()))?;

        if !self.can_user_post_in_board(user_id, &board_slug) {
            return Err(ForumActionError::Auth("user cannot post in this board".to_string()));
        }
        let thread_state = self.thread_state(thread_id);
        if (thread_state.locked || thread_state.closed) && !self.is_moderator_user(user_id) {
            return Err(ForumActionError::Auth("thread is locked".to_string()));
        }

        self.check_limits(user_id, &board_slug, false)?;

        let post = Post {
            id: Uuid::new_v4(),
            thread_id,
            user_id,
            body: trimmed_body.to_string(),
            created_at: now(),
            state: PostState::Allow,
        };

        if self.posts.contains_key(&thread_id) {
            let mod_pending = self.moderation_enabled;
            let content_synced = if let Some(pool) = self.db_pool.clone() {
                let post_for_db = post.clone();
                let inserted = block_in_place(|| {
                    Handle::current().block_on(async move {
                        crate::db::insert_post(&pool, &post_for_db, mod_pending).await
                    })
                });
                match inserted {
                    Ok(()) => true,
                    Err(error) => {
                        if self.requires_postgres() {
                            return Err(self.storage_error(format!(
                                "post insert in postgres failed: {error}"
                            )));
                        }
                        warn!(
                            error = %error,
                            "post insert in postgres failed; falling back to full sync"
                        );
                        false
                    }
                }
            } else {
                false
            };
            if let Some(posts) = self.posts.get_mut(&thread_id) {
                posts.push(post.clone());
            }
            self.notify_mentions_for_content(thread_id, user_id, trimmed_body);
            self.notify_thread_reply(thread_id, post.id, user_id);
            if content_synced {
                self.persist_runtime_state();
            } else {
                self.persist_state();
            }
            return Ok(post);
        }

        Err(ForumActionError::NotFound("thread not found".to_string()))
    }

    pub fn vote_post(
        &mut self,
        post_id: Uuid,
        voter_id: Uuid,
        value: i8,
    ) -> Result<PostScore, ForumActionError> {
        if !matches!(value, -1..=1) {
            return Err(ForumActionError::Validation("invalid vote value".to_string()));
        }

        let thread = self
            .find_post_thread(post_id)
            .ok_or_else(|| ForumActionError::NotFound("post not found".to_string()))?;

        if !self.can_user_vote_on_board(voter_id, &thread.board_slug) {
            return Err(ForumActionError::Auth("user cannot vote right now".to_string()));
        }

        if value == -1 && !self.board_downvoting_allowed(&thread.board_slug) {
            return Err(ForumActionError::Auth(
                "downvotes are disabled for this board".to_string(),
            ));
        }

        let current_vote = self.user_vote_for_post(post_id, voter_id);
        let entry = self.votes.entry(post_id).or_default();
        let next = if value == 0 || current_vote == value { None } else { Some(value) };
        match next {
            Some(v) => {
                entry.insert(voter_id, v);
            }
            None => {
                entry.remove(&voter_id);
            }
        }

        if entry.is_empty() {
            self.votes.remove(&post_id);
        }

        let new_score = self.post_score(post_id);
        if let Some(pool) = self.db_pool.clone() {
            let updated = block_in_place(|| {
                Handle::current().block_on(async move {
                    crate::db::set_vote(&pool, post_id, voter_id, next).await
                })
            });
            match updated {
                Ok(()) => {
                    self.persist_runtime_state();
                    return Ok(PostScore {
                        post_id,
                        user_id: voter_id,
                        score: new_score,
                        thread_id: thread.id,
                    });
                }
                Err(error) => {
                    if self.requires_postgres() {
                        match current_vote {
                            0 => {
                                if let Some(entry) = self.votes.get_mut(&post_id) {
                                    entry.remove(&voter_id);
                                    if entry.is_empty() {
                                        self.votes.remove(&post_id);
                                    }
                                }
                            }
                            previous => {
                                self.votes.entry(post_id).or_default().insert(voter_id, previous);
                            }
                        }
                        return Err(
                            self.storage_error(format!("vote write in postgres failed: {error}"))
                        );
                    }
                    warn!(error = %error, "vote write in postgres failed; falling back to full sync");
                }
            }
        }
        self.persist_state();
        Ok(PostScore { post_id, user_id: voter_id, score: new_score, thread_id: thread.id })
    }

    pub fn moderate_remove_post(
        &mut self,
        thread_id: Uuid,
        post_id: Uuid,
        actor_id: Uuid,
        reason: String,
    ) -> Result<Post, ForumActionError> {
        let Some(actor) = self.users.get(&actor_id) else {
            return Err(ForumActionError::Auth("invalid actor".to_string()));
        };

        if !actor.is_founder() && !matches!(actor.role, UserRole::Mod) {
            return Err(ForumActionError::Auth("only moderators can remove posts".to_string()));
        }

        let Some(posts) = self.posts.get_mut(&thread_id) else {
            return Err(ForumActionError::NotFound("thread not found".to_string()));
        };

        let removed_index = posts
            .iter()
            .position(|post| post.id == post_id)
            .ok_or_else(|| ForumActionError::NotFound("post not found".to_string()))?;

        let removed = posts.remove(removed_index);
        self.votes.remove(&post_id);
        let content_synced = if let Some(pool) = self.db_pool.clone() {
            let deleted = block_in_place(|| {
                Handle::current()
                    .block_on(async move { crate::db::delete_post(&pool, post_id).await })
            });
            match deleted {
                Ok(true) => true,
                Ok(false) => {
                    warn!("post delete in postgres missed row; falling back to full sync");
                    false
                }
                Err(error) => {
                    warn!(
                        error = %error,
                        "post delete in postgres failed; falling back to full sync"
                    );
                    false
                }
            }
        } else {
            false
        };
        let event_synced = self.create_mod_event_removed(
            thread_id,
            Some(post_id),
            actor_id,
            reason.clone(),
            reason.clone(),
        );
        self.add_notification(
            removed.user_id,
            Some(actor_id),
            NotificationKind::PostRemoved,
            Some(thread_id),
            Some(post_id),
            Some(format!(
                "Your post in '{}' was removed by {}.",
                self.thread_title(&thread_id).unwrap_or_else(|| "a thread".to_string()),
                self.user_display_name(actor_id),
            )),
        );
        if content_synced && event_synced {
            self.persist_runtime_state();
        } else {
            self.persist_state();
        }
        Ok(removed)
    }

    pub fn moderate_remove_thread(
        &mut self,
        thread_id: Uuid,
        actor_id: Uuid,
        reason: String,
    ) -> Result<Thread, ForumActionError> {
        let Some(actor) = self.users.get(&actor_id) else {
            return Err(ForumActionError::Auth("invalid actor".to_string()));
        };

        if !actor.is_founder() && !matches!(actor.role, UserRole::Mod) {
            return Err(ForumActionError::Auth("only moderators can remove threads".to_string()));
        }

        let affected_users = self.users_in_thread(thread_id);
        let thread = self
            .threads
            .remove(&thread_id)
            .ok_or_else(|| ForumActionError::NotFound("thread not found".to_string()))?;

        self.threads_by_board.values_mut().for_each(|ids| ids.retain(|id| *id != thread_id));
        if let Some(removed_posts) = self.posts.remove(&thread_id) {
            for post in removed_posts {
                self.votes.remove(&post.id);
            }
        }
        self.thread_tags.remove(&thread_id);
        self.thread_states.remove(&thread_id);
        for hidden in self.hidden_threads.values_mut() {
            hidden.remove(&thread_id);
        }
        for saved in self.saved_threads.values_mut() {
            saved.remove(&thread_id);
        }

        let content_synced = if let Some(pool) = self.db_pool.clone() {
            let deleted = block_in_place(|| {
                Handle::current()
                    .block_on(async move { crate::db::delete_thread(&pool, thread_id).await })
            });
            match deleted {
                Ok(true) => true,
                Ok(false) => {
                    warn!("thread delete in postgres missed row; falling back to full sync");
                    false
                }
                Err(error) => {
                    warn!(
                        error = %error,
                        "thread delete in postgres failed; falling back to full sync"
                    );
                    false
                }
            }
        } else {
            false
        };

        let event_synced = self.create_mod_event_removed(
            thread_id,
            None,
            actor_id,
            reason.clone(),
            reason.clone(),
        );
        for user_id in affected_users {
            self.add_notification(
                user_id,
                Some(actor_id),
                NotificationKind::PostRemoved,
                Some(thread_id),
                None,
                Some(format!(
                    "Thread '{}' was removed by {}.",
                    thread.title,
                    self.user_display_name(actor_id)
                )),
            );
        }

        if content_synced && event_synced {
            self.persist_runtime_state();
        } else {
            self.persist_state();
        }
        Ok(thread)
    }

    fn delete_thread_without_notice(&mut self, thread_id: Uuid) {
        if self.threads.remove(&thread_id).is_none() {
            return;
        }

        self.threads_by_board.values_mut().for_each(|ids| ids.retain(|id| *id != thread_id));
        if let Some(removed_posts) = self.posts.remove(&thread_id) {
            for post in removed_posts {
                self.votes.remove(&post.id);
            }
        }
        self.thread_tags.remove(&thread_id);
        self.thread_states.remove(&thread_id);
        for hidden in self.hidden_threads.values_mut() {
            hidden.remove(&thread_id);
        }
        for saved in self.saved_threads.values_mut() {
            saved.remove(&thread_id);
        }
        self.reports.retain(|report| report.target_thread_id != Some(thread_id));
        self.moderation_events.retain(|event| event.target_thread_id != Some(thread_id));
        for notifications in self.notifications.values_mut() {
            notifications.retain(|item| item.thread_id != Some(thread_id));
        }
    }

    pub fn user_display_name(&self, user_id: Uuid) -> String {
        self.users
            .get(&user_id)
            .map(|user| user.handle.clone())
            .unwrap_or_else(|| "member".to_string())
    }

    pub fn user_id_for_handle(&self, handle: &str) -> Option<Uuid> {
        let expected = handle.trim().trim_start_matches('@').to_ascii_lowercase();
        self.users
            .iter()
            .find(|(_, record)| record.handle.eq_ignore_ascii_case(&expected))
            .map(|(id, _)| *id)
    }

    pub fn boards_exist(&self, board_slug: &str) -> bool {
        self.boards_ttl.contains_key(board_slug)
    }

    pub fn total_threads_by_user(&self, user_id: Uuid) -> usize {
        self.threads.values().filter(|thread| thread.user_id == user_id).count()
    }

    pub fn has_thread(&self, thread_id: &Uuid) -> bool {
        self.threads.contains_key(thread_id)
    }

    pub fn trust_level_for_user(&self, user_id: Uuid) -> u8 {
        let Some(user) = self.users.get(&user_id) else {
            return 0;
        };
        if user.is_founder() {
            return 4;
        }
        if matches!(user.role, UserRole::Mod) {
            return 3;
        }
        let post_count = self.total_posts_by_user(user_id);
        let thread_count = self.total_threads_by_user(user_id);
        if post_count >= 50 || thread_count >= 10 {
            2
        } else if post_count >= 5 || thread_count >= 2 {
            1
        } else {
            0
        }
    }

    pub fn thread_state(&self, thread_id: Uuid) -> ThreadState {
        self.thread_states.get(&thread_id).cloned().unwrap_or_default()
    }

    /// Look up the body of a post by id from the in-memory store. Used by
    /// route handlers that need to forward the body to the moderation
    /// pipeline without holding the forum lock for the duration of an
    /// outbound classifier call. Transient — Phase 11 replaces this with a
    /// `PostRepo.find_by_id` call against Postgres.
    pub fn post_body(&self, post_id: Uuid) -> Option<String> {
        self.posts
            .values()
            .flat_map(|posts| posts.iter())
            .find(|post| post.id == post_id)
            .map(|post| post.body.clone())
    }

    /// Mirror a Postgres-side `mod_state` value into the in-memory post
    /// representation. Transient — exists only for the strangle window
    /// (Phase 9 → Phase 11). After the strangle, the render path reads from
    /// Postgres and this method goes away.
    ///
    /// `pending` is treated as `Allow` because the author still needs to see
    /// their own post while the AI classifier confirms it; the banner does
    /// the "under review" signalling. Quarantine and Removed map straight to
    /// their PostState analogues.
    pub fn mirror_post_mod_state(&mut self, post_id: Uuid, mod_state_db: &str) {
        let target = match mod_state_db {
            "visible" | "pending" => PostState::Allow,
            "quarantined" => PostState::Quarantine,
            "removed" => PostState::Reject,
            _ => return,
        };
        for posts in self.posts.values_mut() {
            if let Some(post) = posts.iter_mut().find(|post| post.id == post_id) {
                post.state = target;
                return;
            }
        }
    }

    pub fn set_thread_pinned(
        &mut self,
        thread_id: Uuid,
        actor_id: Uuid,
        pinned: bool,
    ) -> Result<(), ForumActionError> {
        self.set_thread_state_flag(thread_id, actor_id, |state| state.pinned = pinned)
    }

    pub fn set_thread_locked(
        &mut self,
        thread_id: Uuid,
        actor_id: Uuid,
        locked: bool,
    ) -> Result<(), ForumActionError> {
        self.set_thread_state_flag(thread_id, actor_id, |state| state.locked = locked)
    }

    pub fn set_thread_answered(
        &mut self,
        thread_id: Uuid,
        actor_id: Uuid,
        answered: bool,
    ) -> Result<(), ForumActionError> {
        self.set_thread_state_flag(thread_id, actor_id, |state| state.answered = answered)
    }

    pub fn set_thread_closed(
        &mut self,
        thread_id: Uuid,
        actor_id: Uuid,
        closed: bool,
    ) -> Result<(), ForumActionError> {
        self.set_thread_state_flag(thread_id, actor_id, |state| state.closed = closed)
    }

    fn set_thread_state_flag<F>(
        &mut self,
        thread_id: Uuid,
        actor_id: Uuid,
        update: F,
    ) -> Result<(), ForumActionError>
    where
        F: FnOnce(&mut ThreadState),
    {
        if !self.has_thread(&thread_id) {
            return Err(ForumActionError::NotFound("thread not found".to_string()));
        }
        if !self.is_moderator_user(actor_id) {
            return Err(ForumActionError::Auth(
                "only moderators can change thread state".to_string(),
            ));
        }
        let state = self.thread_states.entry(thread_id).or_default();
        update(state);
        self.create_mod_event_removed(
            thread_id,
            None,
            actor_id,
            "thread state changed".to_string(),
            "thread state changed".to_string(),
        );
        self.persist_state();
        Ok(())
    }

    pub fn tags_for_thread(&self, thread_id: Uuid) -> Vec<String> {
        self.thread_tags.get(&thread_id).cloned().unwrap_or_default()
    }

    pub fn edit_post(
        &mut self,
        post_id: Uuid,
        editor_id: Uuid,
        new_body: String,
    ) -> Result<(), ForumActionError> {
        let new_body = new_body.trim();
        if new_body.is_empty() {
            return Err(ForumActionError::Validation("body is required".to_string()));
        }
        reject_spam_body(new_body)?;
        let can_moderate = self.is_moderator_user(editor_id);
        let Some((_, post)) = self.find_post_mut(post_id) else {
            return Err(ForumActionError::NotFound("post not found".to_string()));
        };
        if post.user_id != editor_id && !can_moderate {
            return Err(ForumActionError::Auth("cannot edit this post".to_string()));
        }
        let previous_body = post.body.clone();
        if previous_body == new_body {
            return Ok(());
        }
        post.body = new_body.to_string();
        self.post_edits.entry(post_id).or_default().push(PostEditRecord {
            post_id,
            editor_id,
            previous_body,
            new_body: new_body.to_string(),
            edited_at: now(),
        });
        self.persist_state();
        Ok(())
    }

    pub fn post_edit_history(&self, post_id: Uuid) -> Vec<PostEditRecord> {
        self.post_edits.get(&post_id).cloned().unwrap_or_default()
    }

    pub fn mute_user(
        &mut self,
        user_id: Uuid,
        muted_user_id: Uuid,
    ) -> Result<(), ForumActionError> {
        if !self.users.contains_key(&user_id) || !self.users.contains_key(&muted_user_id) {
            return Err(ForumActionError::NotFound("user not found".to_string()));
        }
        self.muted_users.entry(user_id).or_default().insert(muted_user_id);
        self.persist_state();
        Ok(())
    }

    pub fn unmute_user(
        &mut self,
        user_id: Uuid,
        muted_user_id: Uuid,
    ) -> Result<(), ForumActionError> {
        if let Some(muted) = self.muted_users.get_mut(&user_id) {
            muted.remove(&muted_user_id);
        }
        self.persist_state();
        Ok(())
    }

    pub fn is_user_muted(&self, user_id: Uuid, actor_id: Uuid) -> bool {
        self.muted_users.get(&user_id).is_some_and(|muted| muted.contains(&actor_id))
    }

    pub fn follow_tag(&mut self, user_id: Uuid, tag: &str) -> Result<(), ForumActionError> {
        if !self.users.contains_key(&user_id) {
            return Err(ForumActionError::NotFound("user not found".to_string()));
        }
        let Some(tag) = normalize_thread_tags(vec![tag.to_string()]).into_iter().next() else {
            return Err(ForumActionError::Validation("tag is required".to_string()));
        };
        self.tag_subscriptions.entry(user_id).or_default().insert(tag);
        self.persist_state();
        Ok(())
    }

    pub fn unfollow_tag(&mut self, user_id: Uuid, tag: &str) -> Result<(), ForumActionError> {
        if !self.users.contains_key(&user_id) {
            return Err(ForumActionError::NotFound("user not found".to_string()));
        }
        let Some(tag) = normalize_thread_tags(vec![tag.to_string()]).into_iter().next() else {
            return Err(ForumActionError::Validation("tag is required".to_string()));
        };
        if let Some(tags) = self.tag_subscriptions.get_mut(&user_id) {
            tags.remove(&tag);
        }
        self.persist_state();
        Ok(())
    }

    pub fn issue_invite(
        &mut self,
        inviter_id: Uuid,
        label: &str,
    ) -> Result<InviteRecord, ForumActionError> {
        if !self.is_moderator_user(inviter_id) && self.trust_level_for_user(inviter_id) < 2 {
            return Err(ForumActionError::Auth("not allowed to invite".to_string()));
        }
        let code = format!("vdni-{}", Uuid::new_v4().simple());
        let invite = InviteRecord {
            code: code.clone(),
            label: label.trim().chars().take(48).collect(),
            inviter_id,
            created_at: now(),
            redeemed_by: None,
            redeemed_at: None,
        };
        self.invites.insert(code, invite.clone());
        self.persist_state();
        Ok(invite)
    }

    pub fn redeem_invite(
        &mut self,
        code: &str,
        handle: &str,
        password: &str,
        ip: Option<String>,
    ) -> Result<ForumUserRecord, ForumActionError> {
        let code = code.trim();
        if self.invites.get(code).is_some_and(|invite| invite.redeemed_by.is_some()) {
            return Err(ForumActionError::Validation("invite already used".to_string()));
        }
        if !self.invites.contains_key(code) {
            return Err(ForumActionError::NotFound("invite not found".to_string()));
        }
        let user = self.create_user(handle, password, ip)?;
        if let Some(invite) = self.invites.get_mut(code) {
            invite.redeemed_by = Some(user.id);
            invite.redeemed_at = Some(now());
        }
        self.persist_state();
        Ok(user)
    }

    pub fn invite_summary(&self) -> Vec<InviteRecord> {
        let mut invites = self.invites.values().cloned().collect::<Vec<_>>();
        invites.sort_by_key(|invite| invite.created_at);
        invites.reverse();
        invites
    }

    pub fn admin_metrics(&self) -> AdminMetrics {
        AdminMetrics {
            total_users: self.users.len(),
            total_threads: self.threads.len(),
            total_posts: self.posts.values().map(Vec::len).sum(),
            open_reports: self.reports.iter().filter(|report| !report.resolved).count(),
            unresolved_notifications: self
                .notifications
                .values()
                .flat_map(|items| items.iter())
                .filter(|notification| !notification.is_read)
                .count(),
        }
    }

    pub fn total_posts_by_user(&self, user_id: Uuid) -> usize {
        self.posts
            .values()
            .map(|posts| posts.iter().filter(|post| post.user_id == user_id).count())
            .sum()
    }

    pub fn all_threads(&self) -> Vec<Thread> {
        if let Some(pool) = self.db_pool.clone() {
            let threads = block_in_place(|| {
                Handle::current().block_on(async move { crate::db::load_all_threads(&pool).await })
            });
            match threads {
                Ok(threads) => return threads,
                Err(error) => {
                    if self.requires_postgres() {
                        return Vec::new();
                    }
                    warn!(error = %error, "all threads load in postgres failed; using local cache");
                }
            }
        }

        let mut threads: Vec<Thread> = self.threads.values().cloned().collect();
        threads.sort_by_key(|thread| thread.created_at);
        threads.reverse();
        threads
    }

    pub fn search(
        &self,
        user_id: Uuid,
        query: &str,
        scope: SearchScope,
        include_hidden: bool,
        limit: usize,
    ) -> Vec<SearchHit> {
        let parsed_query = parse_search_query(query);
        if parsed_query.terms.is_empty() {
            return Vec::new();
        }

        if parsed_query.author_handle.is_none() && parsed_query.tag.is_none() {
            if let Some(pool) = self.db_pool.clone() {
                let search_text = parsed_query.terms.join(" ");
                let board_slug = parsed_query.board_slug.clone();
                let hits = block_in_place(|| {
                    Handle::current().block_on(async move {
                        crate::db::search_forum(
                            &pool,
                            user_id,
                            &search_text,
                            board_slug.as_deref(),
                            scope,
                            include_hidden,
                            limit,
                        )
                        .await
                    })
                });
                match hits {
                    Ok(hits) => return hits,
                    Err(error) => {
                        if self.requires_postgres() {
                            return Vec::new();
                        }
                        warn!(error = %error, "forum search in postgres failed; using local cache");
                    }
                }
            }
        }

        let mut hits = Vec::new();

        for thread in self.all_threads() {
            if parsed_query
                .board_slug
                .as_ref()
                .is_some_and(|board_slug| thread.board_slug != *board_slug)
            {
                continue;
            }

            if !include_hidden && self.is_thread_hidden(user_id, thread.id) {
                continue;
            }
            if parsed_query
                .tag
                .as_ref()
                .is_some_and(|tag| !self.tags_for_thread(thread.id).iter().any(|item| item == tag))
            {
                continue;
            }
            if parsed_query.author_handle.as_ref().is_some_and(|handle| {
                !self.user_display_name(thread.user_id).eq_ignore_ascii_case(handle)
                    && !self
                        .thread_with_posts(&thread.id)
                        .map(|(_, posts)| {
                            posts.iter().any(|post| {
                                self.user_display_name(post.user_id).eq_ignore_ascii_case(handle)
                            })
                        })
                        .unwrap_or(false)
            }) {
                continue;
            }

            let title_lower = thread.title.to_lowercase();
            let tags_lower = self.tags_for_thread(thread.id).join(" ").to_lowercase();
            let posts =
                self.thread_with_posts(&thread.id).map(|(_, posts)| posts).unwrap_or_default();
            let body = posts.first().map(|post| post.body.clone()).unwrap_or_default();
            let body_lower = body.to_lowercase();

            if matches!(scope, SearchScope::All | SearchScope::Threads)
                && text_matches_all_terms(
                    &format!("{title_lower} {tags_lower} {body_lower}"),
                    &parsed_query.terms,
                )
            {
                hits.push(SearchHit {
                    thread_id: thread.id,
                    post_id: None,
                    board_slug: thread.board_slug.clone(),
                    thread_title: thread.title.clone(),
                    author_id: thread.user_id,
                    body_snippet: search_snippet(&body, 180),
                    created_at: thread.created_at,
                    score: self.thread_score(thread.id),
                    kind: SearchHitKind::Thread,
                    is_saved: self.is_thread_saved(user_id, thread.id),
                    is_hidden: self.is_thread_hidden(user_id, thread.id),
                });
            }

            if matches!(scope, SearchScope::All | SearchScope::Posts) {
                for post in posts {
                    if !text_matches_all_terms(&post.body.to_lowercase(), &parsed_query.terms) {
                        continue;
                    }

                    hits.push(SearchHit {
                        thread_id: thread.id,
                        post_id: Some(post.id),
                        board_slug: thread.board_slug.clone(),
                        thread_title: thread.title.clone(),
                        author_id: post.user_id,
                        body_snippet: search_snippet(&post.body, 220),
                        created_at: post.created_at,
                        score: self.post_score(post.id),
                        kind: SearchHitKind::Post,
                        is_saved: self.is_thread_saved(user_id, thread.id),
                        is_hidden: self.is_thread_hidden(user_id, thread.id),
                    });
                }
            }
        }

        hits.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.score.cmp(&a.score))
                .then_with(|| a.thread_title.cmp(&b.thread_title))
        });
        hits.truncate(limit);
        hits
    }

    /// Pull a single user row from PG and upsert it into the in-memory map.
    /// Used after the invite-redemption flow inserts a row directly through
    /// `UserRepo` (bypassing the in-memory store), so subsequent
    /// `verify_user_credentials` calls find the new account without a restart.
    pub async fn refresh_user_from_pg(
        &mut self,
        pool: &PgPool,
        user_id: Uuid,
    ) -> Result<(), sqlx::Error> {
        let row = crate::repository::users::UserRepo.find_by_id(pool, user_id).await?;
        let Some(row) = row else {
            return Ok(());
        };
        let role = match row.role.as_str() {
            "founder" => UserRole::Founder,
            "mod" => UserRole::Mod,
            _ => UserRole::Member,
        };
        let user = ForumUserRecord {
            id: row.id,
            handle: row.handle,
            password_hash: row.password_hash,
            role,
            is_banned: row.banned,
            probation_until: row.probation_until,
            created_at: row.created_at,
            last_known_ip: row.last_known_ip,
        };
        self.users.insert(row.id, user);
        Ok(())
    }

    pub fn verify_user_credentials(&mut self, handle: &str, password: &str) -> Option<Uuid> {
        let expected = handle.trim();
        let mut authenticated_user_id = None;
        for (id, user) in &self.users {
            if user.handle.eq_ignore_ascii_case(expected)
                && !user.is_banned
                && verify_password(&user.handle, password, &user.password_hash)
            {
                authenticated_user_id = Some(*id);
                break;
            }
        }
        let user_id = authenticated_user_id?;
        if self.users.get(&user_id).is_some_and(|user| !user.password_hash.starts_with("$argon2")) {
            if let Some(user) = self.users.get_mut(&user_id) {
                user.password_hash = hash_password(password);
            }
            if let Some(pool) = self.db_pool.clone() {
                if let Some(user) = self.users.get(&user_id).cloned() {
                    let updated = block_in_place(|| {
                        Handle::current()
                            .block_on(async move { crate::db::upsert_user(&pool, &user).await })
                    });
                    if let Err(error) = updated {
                        warn!(
                            error = %error,
                            "password hash upgrade in postgres failed; falling back to full sync"
                        );
                        self.persist_state();
                    } else {
                        self.persist_runtime_state();
                    }
                }
            } else {
                self.persist_state();
            }
        }
        Some(user_id)
    }

    pub fn founder_uses_default_password(&self) -> bool {
        self.users
            .get(&ROOT_USER_ID)
            .is_some_and(|user| verify_password(&user.handle, ROOT_HANDLE, &user.password_hash))
    }

    pub fn set_founder_password(&mut self, password: &str) -> Result<(), ForumActionError> {
        if password.trim().len() < 8 {
            return Err(ForumActionError::Validation(
                "founder password must be at least 8 characters".to_string(),
            ));
        }

        let root = self
            .users
            .get_mut(&ROOT_USER_ID)
            .ok_or_else(|| ForumActionError::NotFound("founder account not found".to_string()))?;
        root.password_hash = hash_password(password.trim());
        root.role = UserRole::Founder;

        if let Some(pool) = self.db_pool.clone() {
            let root_for_db = root.clone();
            let updated = block_in_place(|| {
                Handle::current()
                    .block_on(async move { crate::db::upsert_user(&pool, &root_for_db).await })
            });
            if let Err(error) = updated {
                warn!(
                    error = %error,
                    "founder password write in postgres failed; falling back to full sync"
                );
                self.persist_state();
            } else {
                self.persist_runtime_state();
            }
        } else {
            self.persist_state();
        }

        Ok(())
    }

    pub fn create_user(
        &mut self,
        handle: &str,
        password: &str,
        ip: Option<String>,
    ) -> Result<ForumUserRecord, ForumActionError> {
        let throttle_key =
            ip.clone().unwrap_or_else(|| format!("handle:{}", handle.trim().to_ascii_lowercase()));
        self.prune_registration_attempts();
        if self.registration_attempts.get(&throttle_key).map_or(0, |attempts| attempts.len())
            >= MAX_REGISTRATIONS_PER_WINDOW
        {
            return Err(ForumActionError::RateLimit(
                "registration limit reached, try again later".to_string(),
            ));
        }

        let normalized = normalize_handle(handle)?;
        if password.trim().len() < 8 {
            return Err(ForumActionError::Validation(
                "password must be at least 8 characters".to_string(),
            ));
        }
        if self.user_id_for_handle(&normalized).is_some() {
            return Err(ForumActionError::Validation("handle already exists".to_string()));
        }
        if ip.as_ref().is_some_and(|value| self.banned_ips.contains(value)) {
            return Err(ForumActionError::Auth("registration blocked".to_string()));
        }

        let user = ForumUserRecord {
            id: Uuid::new_v4(),
            handle: normalized.clone(),
            password_hash: hash_password(password.trim()),
            role: UserRole::Member,
            is_banned: false,
            probation_until: None,
            created_at: now(),
            last_known_ip: ip,
        };
        self.registration_attempts.entry(throttle_key).or_default().push(now());
        let default_settings = UserProfileSettings::default();
        if let Some(pool) = self.db_pool.clone() {
            let user_for_db = user.clone();
            let user_id = user.id;
            let default_settings_for_db = default_settings.clone();
            let inserted = block_in_place(|| {
                Handle::current().block_on(async move {
                    crate::db::upsert_user(&pool, &user_for_db).await?;
                    crate::db::upsert_user_settings(&pool, user_id, &default_settings_for_db).await
                })
            });
            match inserted {
                Ok(()) => {
                    self.users.insert(user.id, user.clone());
                    self.user_settings.insert(user.id, default_settings);
                    self.persist_runtime_state();
                    return Ok(user);
                }
                Err(error) => {
                    if self.requires_postgres() {
                        return Err(
                            self.storage_error(format!("user insert in postgres failed: {error}"))
                        );
                    }
                    warn!(error = %error, "user insert in postgres failed; falling back to full sync");
                }
            }
        }
        self.users.insert(user.id, user.clone());
        self.user_settings.insert(user.id, default_settings);
        self.persist_state();
        Ok(user)
    }

    pub fn login_allowed(&mut self, key: &str) -> bool {
        self.prune_login_attempts();
        self.failed_login_attempts.get(key).map_or(0, |attempts| attempts.len())
            < MAX_FAILED_LOGINS_PER_WINDOW
    }

    pub fn record_failed_login(&mut self, key: &str) {
        self.prune_login_attempts();
        self.failed_login_attempts.entry(key.to_string()).or_default().push(now());
    }

    pub fn clear_failed_logins(&mut self, key: &str) {
        self.failed_login_attempts.remove(key);
    }

    pub fn user_role_name(&self, user_id: Uuid) -> &'static str {
        self.users
            .get(&user_id)
            .map(|user| match user.role {
                UserRole::Founder => "founder",
                UserRole::Mod => "mod",
                UserRole::Member => "member",
            })
            .unwrap_or("member")
    }

    pub fn can_moderate(&self, user_id: Uuid) -> bool {
        self.users
            .get(&user_id)
            .is_some_and(|user| matches!(user.role, UserRole::Founder | UserRole::Mod))
    }

    pub fn is_founder(&self, user_id: Uuid) -> bool {
        self.users.get(&user_id).is_some_and(ForumUserRecord::is_founder)
    }

    pub fn admin_user_summaries(&self) -> Vec<AdminUserSummary> {
        if let Some(pool) = self.db_pool.clone() {
            let users = block_in_place(|| {
                Handle::current()
                    .block_on(async move { crate::db::list_admin_user_summaries(&pool).await })
            });
            match users {
                Ok(users) => return users,
                Err(error) => {
                    if self.requires_postgres() {
                        return Vec::new();
                    }
                    warn!(error = %error, "admin users in postgres failed; using local cache");
                }
            }
        }

        let mut users = self
            .users
            .values()
            .map(|user| AdminUserSummary {
                id: user.id,
                handle: user.handle.clone(),
                role: self.user_role_name(user.id).to_string(),
                is_banned: user.is_banned,
                thread_count: self.total_threads_by_user(user.id),
                post_count: self.total_posts_by_user(user.id),
                can_manage: !user.is_founder(),
            })
            .collect::<Vec<_>>();

        users.sort_by(|left, right| left.handle.cmp(&right.handle));
        users.sort_by_key(|user| if user.role == "founder" { 0 } else { 1 });
        users
    }

    pub fn banned_ips(&self) -> &HashSet<String> {
        &self.banned_ips
    }

    pub fn set_last_known_ip(
        &mut self,
        target_user_id: Uuid,
        ip: String,
    ) -> Result<(), ForumActionError> {
        let Some(user) = self.users.get_mut(&target_user_id) else {
            return Err(ForumActionError::NotFound("user not found".to_string()));
        };
        user.last_known_ip = Some(ip);
        let user = user.clone();
        if let Some(pool) = self.db_pool.clone() {
            let updated = block_in_place(|| {
                Handle::current()
                    .block_on(async move { crate::db::upsert_user(&pool, &user).await })
            });
            match updated {
                Ok(()) => {
                    self.persist_runtime_state();
                    return Ok(());
                }
                Err(error) => {
                    warn!(
                        error = %error,
                        "last known ip write in postgres failed; falling back to full sync"
                    );
                }
            }
        }
        self.persist_state();
        Ok(())
    }

    pub fn wipe_user_content(
        &mut self,
        actor_id: Uuid,
        target_user_id: Uuid,
    ) -> Result<ForumUserRecord, ForumActionError> {
        let Some(actor) = self.users.get(&actor_id) else {
            return Err(ForumActionError::Auth("invalid actor".to_string()));
        };
        if !actor.is_founder() {
            return Err(ForumActionError::Auth("only founders can manage users".to_string()));
        }

        let Some(target) = self.users.get(&target_user_id).cloned() else {
            return Err(ForumActionError::NotFound("user not found".to_string()));
        };
        if target.is_founder() {
            return Err(ForumActionError::Auth("founder account cannot be wiped".to_string()));
        }

        self.purge_user_content(target_user_id);
        if self.sync_admin_user_mutation_to_postgres() {
            self.persist_runtime_state();
        } else {
            self.persist_state();
        }
        Ok(target)
    }

    pub fn delete_user(
        &mut self,
        actor_id: Uuid,
        target_user_id: Uuid,
    ) -> Result<ForumUserRecord, ForumActionError> {
        let Some(actor) = self.users.get(&actor_id) else {
            return Err(ForumActionError::Auth("invalid actor".to_string()));
        };
        if !actor.is_founder() {
            return Err(ForumActionError::Auth("only founders can manage users".to_string()));
        }

        let Some(target) = self.users.get(&target_user_id).cloned() else {
            return Err(ForumActionError::NotFound("user not found".to_string()));
        };
        if target.is_founder() {
            return Err(ForumActionError::Auth("founder account cannot be deleted".to_string()));
        }

        self.purge_user_content(target_user_id);

        if let Some(ip) = target.last_known_ip.clone() {
            self.banned_ips.insert(ip);
        }

        self.user_settings.remove(&target_user_id);
        self.board_subscriptions.remove(&target_user_id);
        self.hidden_threads.remove(&target_user_id);
        self.saved_threads.remove(&target_user_id);

        self.sessions.retain(|_, session| session.user_id != target_user_id);
        self.api_tokens.retain(|_, record| record.user_id != target_user_id);
        self.reports.retain(|report| report.actor_id != target_user_id);
        self.moderation_events.retain(|event| event.actor_id != target_user_id);
        self.users.remove(&target_user_id);

        if self.sync_admin_user_mutation_to_postgres() {
            self.persist_runtime_state();
        } else {
            self.persist_state();
        }
        Ok(target)
    }

    fn purge_user_content(&mut self, target_user_id: Uuid) {
        let deleted_thread_ids = self
            .threads
            .values()
            .filter(|thread| thread.user_id == target_user_id)
            .map(|thread| thread.id)
            .collect::<Vec<_>>();
        for thread_id in deleted_thread_ids {
            self.delete_thread_without_notice(thread_id);
        }

        let mut deleted_post_ids = Vec::new();
        let mut empty_thread_ids = Vec::new();
        for (thread_id, posts) in self.posts.iter_mut() {
            let original_len = posts.len();
            posts.retain(|post| {
                if post.user_id == target_user_id {
                    deleted_post_ids.push(post.id);
                    false
                } else {
                    true
                }
            });
            if posts.is_empty() && original_len > 0 {
                empty_thread_ids.push(*thread_id);
            }
        }
        for post_id in &deleted_post_ids {
            self.votes.remove(post_id);
        }
        for notifications in self.notifications.values_mut() {
            notifications.retain(|item| {
                item.post_id.map(|id| !deleted_post_ids.contains(&id)).unwrap_or(true)
            });
        }
        self.reports.retain(|report| {
            report.target_post_id.map(|id| !deleted_post_ids.contains(&id)).unwrap_or(true)
        });
        self.moderation_events.retain(|event| {
            event.target_post_id.map(|id| !deleted_post_ids.contains(&id)).unwrap_or(true)
        });
        for thread_id in empty_thread_ids {
            self.delete_thread_without_notice(thread_id);
        }

        for votes in self.votes.values_mut() {
            votes.remove(&target_user_id);
        }
        self.votes.retain(|_, votes| !votes.is_empty());

        self.notifications.remove(&target_user_id);
        for notifications in self.notifications.values_mut() {
            notifications.retain(|item| item.actor_id != Some(target_user_id));
        }
    }

    pub fn create_session(&mut self, user_id: Uuid) -> Option<String> {
        let user = self.users.get(&user_id)?;
        if user.is_banned {
            return None;
        }

        let token = make_session_token();
        let token_digest = self.token_storage_digest(&token).for_storage();
        let csrf = Uuid::new_v4().to_string();
        let record = SessionRecord {
            user_id,
            created_at: now(),
            expires_at: now() + Duration::days(7),
            csrf_token: csrf,
        };
        if let Some(pool) = self.db_pool.clone() {
            let session_record = record.clone();
            let token_for_db = token.clone();
            let token_pepper = self.token_pepper.clone();
            let inserted = block_in_place(|| {
                Handle::current().block_on(async move {
                    crate::db::insert_session(
                        &pool,
                        &token_for_db,
                        token_pepper.as_deref(),
                        &session_record,
                    )
                    .await
                })
            });
            if let Err(error) = inserted {
                self.record_storage_failure("session insert in postgres failed", &error);
                warn!(error = %error, "session insert in postgres failed");
                return None;
            }
            self.sessions.insert(token_digest, record);
            self.persist_runtime_state();
            return Some(token);
        }

        self.sessions.insert(token_digest, record);
        self.persist_state();
        Some(token)
    }

    pub fn destroy_session(&mut self, token: &str) -> bool {
        if let Some(pool) = self.db_pool.clone() {
            let token_pepper = self.token_pepper.clone();
            let deleted = block_in_place(|| {
                Handle::current().block_on(async move {
                    crate::db::delete_session(&pool, token, token_pepper.as_deref()).await
                })
            });
            if self.requires_postgres() {
                match deleted {
                    Ok(db_removed) => {
                        let removed = self.remove_session_by_raw_token(token);
                        if removed || db_removed {
                            self.persist_runtime_state();
                        }
                        return removed || db_removed;
                    }
                    Err(error) => {
                        self.record_storage_failure("session delete in postgres failed", &error);
                        return false;
                    }
                }
            }
            let removed = self.remove_session_by_raw_token(token);
            match deleted {
                Ok(db_removed) => {
                    if removed || db_removed {
                        self.persist_runtime_state();
                    }
                    return removed || db_removed;
                }
                Err(error) => {
                    warn!(error = %error, "session delete in postgres failed; clearing local cache only");
                    if removed {
                        self.persist_runtime_state();
                    }
                    return removed;
                }
            }
        }

        let removed = self.remove_session_by_raw_token(token);
        if removed {
            self.persist_state();
        }
        removed
    }

    pub fn get_session_user(&mut self, token: &str) -> Option<Uuid> {
        if let Some(pool) = self.db_pool.clone() {
            let token_pepper = self.token_pepper.clone();
            let db_session = block_in_place(|| {
                Handle::current().block_on(async move {
                    crate::db::load_session(&pool, token, token_pepper.as_deref()).await
                })
            });
            match db_session {
                Ok(Some(session)) => {
                    if session.expires_at < now() {
                        self.remove_session_by_raw_token(token);
                        self.persist_state();
                        return None;
                    }
                    self.sessions
                        .insert(self.token_storage_digest(token).for_storage(), session.clone());
                    return Some(session.user_id);
                }
                Ok(None) => {
                    self.remove_session_by_raw_token(token);
                    return None;
                }
                Err(error) => {
                    if self.requires_postgres() {
                        self.record_storage_failure("session lookup in postgres failed", &error);
                        return None;
                    }
                    warn!(error = %error, "session lookup in postgres failed; using local cache");
                }
            }
        }

        let session = self
            .token_lookup_digests(token)
            .into_iter()
            .find_map(|digest| self.sessions.get(&digest))?;
        if session.expires_at < now() {
            self.remove_session_by_raw_token(token);
            self.persist_state();
            return None;
        }
        Some(session.user_id)
    }

    pub fn csrf_token_for_session(&mut self, token: &str) -> Option<String> {
        if let Some(pool) = self.db_pool.clone() {
            let token_pepper = self.token_pepper.clone();
            let db_session = block_in_place(|| {
                Handle::current().block_on(async move {
                    crate::db::load_session(&pool, token, token_pepper.as_deref()).await
                })
            });
            match db_session {
                Ok(Some(session)) => return Some(session.csrf_token),
                Ok(None) => return None,
                Err(error) => {
                    if self.requires_postgres() {
                        self.record_storage_failure("csrf lookup in postgres failed", &error);
                        return None;
                    }
                    warn!(error = %error, "csrf lookup in postgres failed; using local cache");
                }
            }
        }

        self.token_lookup_digests(token)
            .into_iter()
            .find_map(|digest| self.sessions.get(&digest).map(|session| session.csrf_token.clone()))
    }

    pub fn validate_csrf(&mut self, token: &str, csrf: &str) -> bool {
        self.csrf_token_for_session(token).is_some_and(|expected| expected == csrf)
    }

    pub fn session_cookie_name() -> &'static str {
        "viceden_session"
    }

    pub fn issue_api_token(&mut self, user_id: Uuid) -> Option<String> {
        self.issue_named_api_token(user_id, "API token".to_string()).map(|issued| issued.token)
    }

    pub fn issue_named_api_token(&mut self, user_id: Uuid, name: String) -> Option<IssuedApiToken> {
        if !self.users.contains_key(&user_id)
            || self.users.get(&user_id).is_some_and(|user| user.is_banned)
        {
            return None;
        }
        let token = auth::tokens::new_pat_token();
        let created_at = now();
        let preview = format!("{}...", token.chars().take(12).collect::<String>());
        let token_digest = self.token_storage_digest(&token).for_storage();
        let record = ApiTokenRecord {
            id: Uuid::new_v4(),
            user_id,
            name: name.trim().chars().take(40).collect(),
            preview: preview.clone(),
            created_at,
            last_used_at: None,
        };
        if let Some(pool) = self.db_pool.clone() {
            let token_for_db = token.clone();
            let record_for_db = record.clone();
            let token_pepper = self.token_pepper.clone();
            let inserted = block_in_place(|| {
                Handle::current().block_on(async move {
                    crate::db::insert_api_token(
                        &pool,
                        &token_for_db,
                        token_pepper.as_deref(),
                        &record_for_db,
                    )
                    .await
                })
            });
            if let Err(error) = inserted {
                self.record_storage_failure("api token insert in postgres failed", &error);
                warn!(error = %error, "api token insert in postgres failed");
                return None;
            }
            self.api_tokens.insert(token_digest, record.clone());
            self.persist_runtime_state();
            return Some(IssuedApiToken {
                id: record.id,
                name: record.name,
                token,
                preview,
                created_at,
            });
        }

        self.api_tokens.insert(token_digest, record.clone());
        self.persist_state();
        Some(IssuedApiToken { id: record.id, name: record.name, token, preview, created_at })
    }

    pub fn api_tokens_for_user(&mut self, user_id: Uuid) -> Vec<ApiTokenSummary> {
        if let Some(pool) = self.db_pool.clone() {
            let db_tokens = block_in_place(|| {
                Handle::current().block_on(async move {
                    crate::db::list_api_tokens_for_user(&pool, user_id).await
                })
            });
            match db_tokens {
                Ok(tokens) => return tokens,
                Err(error) => {
                    if self.requires_postgres() {
                        self.record_storage_failure("api token list in postgres failed", &error);
                        return Vec::new();
                    }
                    warn!(error = %error, "api token list in postgres failed; using local cache");
                }
            }
        }

        let mut items = self
            .api_tokens
            .iter()
            .filter(|(_, record)| record.user_id == user_id)
            .map(|(_, record)| ApiTokenSummary {
                id: record.id,
                name: record.name.clone(),
                preview: record.preview.clone(),
                created_at: record.created_at,
                last_used_at: record.last_used_at,
            })
            .collect::<Vec<_>>();
        items.sort_by_key(|item| item.created_at);
        items.reverse();
        items
    }

    pub fn resolve_api_token(&mut self, token: &str) -> Option<Uuid> {
        if let Some(pool) = self.db_pool.clone() {
            let token_pepper = self.token_pepper.clone();
            let resolved = block_in_place(|| {
                Handle::current().block_on(async move {
                    crate::db::resolve_api_token_user(&pool, token, token_pepper.as_deref()).await
                })
            });
            match resolved {
                Ok(Some(user_id)) => {
                    let mut touched = false;
                    for digest in self.token_lookup_digests(token) {
                        if let Some(record) = self.api_tokens.get_mut(&digest) {
                            record.last_used_at = Some(now());
                            touched = true;
                        }
                    }
                    if touched {
                        self.persist_runtime_state();
                    }
                    return Some(user_id);
                }
                Ok(None) => return None,
                Err(error) => {
                    if self.requires_postgres() {
                        self.record_storage_failure("api token lookup in postgres failed", &error);
                        return None;
                    }
                    warn!(error = %error, "api token lookup in postgres failed; using local cache");
                }
            }
        }

        let digest = self
            .token_lookup_digests(token)
            .into_iter()
            .find(|digest| self.api_tokens.contains_key(digest))?;
        let record = self.api_tokens.get_mut(&digest)?;
        record.last_used_at = Some(now());
        let user_id = record.user_id;
        self.persist_state();
        Some(user_id)
    }

    pub fn revoke_api_token(&mut self, user_id: Uuid, token: &str) -> bool {
        if let Some(pool) = self.db_pool.clone() {
            let token_pepper = self.token_pepper.clone();
            let deleted = block_in_place(|| {
                Handle::current().block_on(async move {
                    crate::db::delete_api_token(&pool, user_id, token, token_pepper.as_deref())
                        .await
                })
            });
            match deleted {
                Ok(true) => {
                    self.remove_api_token_by_raw_token(token);
                    self.persist_runtime_state();
                    return true;
                }
                Ok(false) => return false,
                Err(error) => {
                    if self.requires_postgres() {
                        self.record_storage_failure("api token delete in postgres failed", &error);
                    }
                    warn!(error = %error, "api token delete in postgres failed");
                    return false;
                }
            }
        }

        let digest = match self.token_lookup_digests(token).into_iter().find(|digest| {
            self.api_tokens.get(digest).is_some_and(|record| record.user_id == user_id)
        }) {
            Some(digest) => digest,
            None => return false,
        };
        if self.api_tokens.remove(&digest).is_none() {
            return false;
        }
        self.persist_state();
        true
    }

    pub fn revoke_api_token_by_id(&mut self, user_id: Uuid, token_id: Uuid) -> bool {
        if let Some(pool) = self.db_pool.clone() {
            let deleted = block_in_place(|| {
                Handle::current().block_on(async move {
                    crate::db::delete_api_token_by_id(&pool, user_id, token_id).await
                })
            });
            match deleted {
                Ok(true) => {
                    let token = self.api_tokens.iter().find_map(|(token, record)| {
                        (record.user_id == user_id && record.id == token_id)
                            .then_some(token.clone())
                    });
                    if let Some(token) = token {
                        self.api_tokens.remove(&token);
                    }
                    self.persist_runtime_state();
                    return true;
                }
                Ok(false) => return false,
                Err(error) => {
                    if self.requires_postgres() {
                        self.record_storage_failure(
                            "api token delete by id in postgres failed",
                            &error,
                        );
                    }
                    warn!(error = %error, "api token delete by id in postgres failed");
                    return false;
                }
            }
        }

        let token = self.api_tokens.iter().find_map(|(token, record)| {
            (record.user_id == user_id && record.id == token_id).then_some(token.clone())
        });
        let Some(token) = token else {
            return false;
        };
        self.api_tokens.remove(&token);
        self.persist_state();
        true
    }

    pub fn thread_id_for_post(&self, post_id: Uuid) -> Option<Uuid> {
        self.posts.iter().find_map(|(thread_id, posts)| {
            posts.iter().any(|post| post.id == post_id).then_some(*thread_id)
        })
    }

    pub fn downvoting_allowed_for_board(&self, board_slug: &str) -> bool {
        self.board_downvote_enabled.get(board_slug).copied().unwrap_or(true)
    }

    pub fn purge_expired(&mut self) {
        self.remove_expired_sessions();
        self.purge_expired_threads();
    }

    pub fn purge_expired_threads(&mut self) {
        let now = now();
        let mut thread_ids_to_remove = Vec::new();
        let mut vote_ids_to_remove = Vec::new();
        let mut post_count_removed = 0usize;

        for (thread_id, thread) in &self.threads {
            if thread.expires_at > now {
                continue;
            }

            let posts = self.posts.entry(*thread_id).or_default();
            if posts.len() <= THREAD_POSTS_KEEP_LARGE_THREAD {
                thread_ids_to_remove.push(*thread_id);
                continue;
            }

            let first = posts.first().map(|post| post.id);
            let before = posts.len();
            let before_ids = posts.iter().map(|post| post.id).collect::<Vec<_>>();
            posts.retain(|post| {
                if Some(post.id) == first {
                    return true;
                }
                post.created_at > thread.expires_at
            });

            if let Some(first_id) = first {
                posts.retain(|post| post.id == first_id || post.created_at > thread.expires_at);
            }
            let kept_ids: HashSet<Uuid> = posts.iter().map(|post| post.id).collect();
            for post_id in before_ids {
                if !kept_ids.contains(&post_id) {
                    vote_ids_to_remove.push(post_id);
                }
            }
            let after = posts.len();
            post_count_removed += before.saturating_sub(after);

            if posts.is_empty() {
                thread_ids_to_remove.push(*thread_id);
            }
        }

        for post_id in &vote_ids_to_remove {
            self.votes.remove(post_id);
        }

        let removed_thread_count = thread_ids_to_remove.len();
        let mut direct_sql_ok = true;
        if let Some(pool) = self.db_pool.clone() {
            for post_id in vote_ids_to_remove.iter().copied() {
                let pool = pool.clone();
                let deleted = block_in_place(|| {
                    Handle::current()
                        .block_on(async move { crate::db::delete_post(&pool, post_id).await })
                });
                match deleted {
                    Ok(_) => {}
                    Err(error) => {
                        warn!(
                            error = %error,
                            "expired post delete in postgres failed; falling back to full sync"
                        );
                        direct_sql_ok = false;
                        break;
                    }
                }
            }
        } else {
            direct_sql_ok = false;
        }

        for id in thread_ids_to_remove {
            if direct_sql_ok {
                if let Some(pool) = self.db_pool.clone() {
                    let deleted = block_in_place(|| {
                        Handle::current()
                            .block_on(async move { crate::db::delete_thread(&pool, id).await })
                    });
                    if let Err(error) = deleted {
                        warn!(
                            error = %error,
                            "expired thread delete in postgres failed; falling back to full sync"
                        );
                        direct_sql_ok = false;
                    }
                }
            }

            self.threads.remove(&id);
            if let Some(removed_posts) = self.posts.remove(&id) {
                for post in removed_posts {
                    self.votes.remove(&post.id);
                }
            }
            self.thread_tags.remove(&id);
            self.thread_states.remove(&id);
            for hidden in self.hidden_threads.values_mut() {
                hidden.remove(&id);
            }
            for saved in self.saved_threads.values_mut() {
                saved.remove(&id);
            }
            self.threads_by_board.values_mut().for_each(|ids| ids.retain(|value| *value != id));
        }

        if post_count_removed > 0 || removed_thread_count > 0 {
            if direct_sql_ok {
                self.persist_runtime_state();
            } else {
                self.persist_state();
            }
        }
    }

    pub fn post_score(&self, post_id: Uuid) -> i32 {
        self.votes
            .get(&post_id)
            .map(|votes| votes.values().copied().sum::<i8>() as i32)
            .unwrap_or(0)
    }

    pub fn user_vote_for_post(&self, post_id: Uuid, user_id: Uuid) -> i8 {
        self.votes.get(&post_id).and_then(|votes| votes.get(&user_id).copied()).unwrap_or(0)
    }

    pub fn thread_score(&self, thread_id: Uuid) -> i32 {
        self.posts
            .get(&thread_id)
            .map(|posts| posts.iter().map(|post| self.post_score(post.id)).sum())
            .unwrap_or(0)
    }

    fn thread_last_activity(&self, thread_id: Uuid) -> Option<DateTime<Utc>> {
        self.thread_with_posts(&thread_id)
            .and_then(|(_, posts)| posts.last().map(|post| post.created_at))
    }

    fn thread_board_slug(&self, thread_id: Uuid) -> Option<String> {
        self.threads.get(&thread_id).map(|thread| thread.board_slug.clone())
    }

    fn users_in_thread(&self, thread_id: Uuid) -> Vec<Uuid> {
        let mut users = Vec::new();
        let mut seen = HashSet::new();
        if let Some((thread, posts)) = self.thread_with_posts(&thread_id) {
            if seen.insert(thread.user_id) {
                users.push(thread.user_id);
            }
            for post in posts {
                if seen.insert(post.user_id) {
                    users.push(post.user_id);
                }
            }
        }
        users
    }

    fn boards_ttl(&self, board_slug: &str) -> Option<u32> {
        if board_slug.trim().is_empty() {
            None
        } else {
            self.boards_ttl.get(board_slug).copied()
        }
    }

    fn board_downvoting_allowed(&self, board_slug: &str) -> bool {
        self.board_downvote_enabled.get(board_slug).copied().unwrap_or(true)
    }

    fn can_user_post_in_board(&self, user_id: Uuid, board_slug: &str) -> bool {
        self.users
            .get(&user_id)
            .is_some_and(|user| self.boards_exist(board_slug) && !user.is_banned)
    }

    fn can_user_vote_on_board(&self, user_id: Uuid, board_slug: &str) -> bool {
        self.users
            .get(&user_id)
            .is_some_and(|user| self.boards_exist(board_slug) && !user.is_banned)
    }

    fn is_moderator_user(&self, user_id: Uuid) -> bool {
        self.users
            .get(&user_id)
            .is_some_and(|user| user.is_founder() || matches!(user.role, UserRole::Mod))
    }

    pub fn check_limits(
        &self,
        user_id: Uuid,
        board_slug: &str,
        is_thread: bool,
    ) -> Result<(), ForumActionError> {
        let Some(user) = self.users.get(&user_id) else {
            return Err(ForumActionError::Auth("user not found".to_string()));
        };

        if user_id != ROOT_USER_ID {
            if let Some(until) = user.probation_until {
                if until > now() {
                    return Err(ForumActionError::Auth("probation active".to_string()));
                }
            }

            let day_start = now().date_naive();
            let this_user_threads = self
                .threads
                .values()
                .filter(|thread| {
                    thread.user_id == user_id && thread.created_at.date_naive() == day_start
                })
                .count();
            if is_thread && this_user_threads >= 5 {
                return Err(ForumActionError::RateLimit("thread limit reached".to_string()));
            }

            let replies_today = self
                .posts
                .values()
                .flat_map(|posts| {
                    posts
                        .iter()
                        .filter(|post| post.user_id == user_id)
                        .filter(|post| post.created_at.date_naive() == day_start)
                })
                .count();
            if !is_thread && replies_today >= 50 {
                return Err(ForumActionError::RateLimit("reply limit reached".to_string()));
            }

            if board_slug == "anonymous"
                && thread_count_for_user_in_board(self, user_id, board_slug) > 0
                && is_thread
            {
                return Err(ForumActionError::Validation(
                    "one thread limit per day for this board".to_string(),
                ));
            }
        }

        Ok(())
    }

    fn find_post_thread(&self, post_id: Uuid) -> Option<Thread> {
        self.posts
            .iter()
            .find_map(|(thread_id, posts)| {
                posts.iter().any(|post| post.id == post_id).then(|| self.threads.get(thread_id))
            })
            .and_then(|thread| thread.cloned())
    }

    fn find_post_mut(&mut self, post_id: Uuid) -> Option<(Uuid, &mut Post)> {
        self.posts.iter_mut().find_map(|(thread_id, posts)| {
            posts.iter_mut().find(|post| post.id == post_id).map(|post| (*thread_id, post))
        })
    }

    fn create_mod_event_removed(
        &mut self,
        thread_id: Uuid,
        post_id: Option<Uuid>,
        actor_id: Uuid,
        action: String,
        note: String,
    ) -> bool {
        let event = ModerationEvent {
            id: Uuid::new_v4(),
            actor_id,
            target_post_id: post_id,
            target_thread_id: Some(thread_id),
            reason: action,
            created_at: now(),
        };
        self.moderation_events.push(event);
        let _ = note;
        if let Some(pool) = self.db_pool.clone() {
            let event = self.moderation_events.last().cloned().expect("event was just pushed");
            let inserted = block_in_place(|| {
                Handle::current().block_on(async move {
                    crate::db::insert_moderation_event(&pool, &event).await
                })
            });
            match inserted {
                Ok(()) => return true,
                Err(error) => {
                    warn!(
                        error = %error,
                        "moderation event insert in postgres failed; falling back to full sync"
                    );
                    return false;
                }
            }
        }
        false
    }

    fn remove_expired_sessions(&mut self) {
        let now = now();
        let expired: Vec<_> = self
            .sessions
            .iter()
            .filter_map(|(token, session)| (session.expires_at < now).then_some(token.clone()))
            .collect();
        for token in expired {
            self.sessions.remove(&token);
        }
    }

    fn prune_login_attempts(&mut self) {
        prune_attempt_map(
            &mut self.failed_login_attempts,
            now() - Duration::minutes(LOGIN_THROTTLE_WINDOW_MINUTES),
        );
    }

    fn prune_registration_attempts(&mut self) {
        prune_attempt_map(
            &mut self.registration_attempts,
            now() - Duration::minutes(REGISTRATION_THROTTLE_WINDOW_MINUTES),
        );
    }
}

#[derive(Clone)]
pub struct UserActivityTeaser {
    pub board_slug: String,
    pub thread_id: Uuid,
    pub post_id: Uuid,
    pub thread_title: String,
    pub body_snippet: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct ThreadPreview {
    pub thread: Thread,
    pub score: i32,
    pub reply_count: usize,
    pub score_hidden: bool,
    pub last_activity: DateTime<Utc>,
}

#[derive(Clone)]
pub struct PostScore {
    pub post_id: Uuid,
    pub thread_id: Uuid,
    pub user_id: Uuid,
    pub score: i32,
}

#[derive(Clone)]
pub struct AdminUserSummary {
    pub id: Uuid,
    pub handle: String,
    pub role: String,
    pub is_banned: bool,
    pub thread_count: usize,
    pub post_count: usize,
    pub can_manage: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchScope {
    All,
    Threads,
    Posts,
}

impl SearchScope {
    pub fn from_query(raw: Option<&str>) -> Self {
        match raw.unwrap_or("all").trim().to_ascii_lowercase().as_str() {
            "threads" => Self::Threads,
            "posts" => Self::Posts,
            _ => Self::All,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Threads => "threads",
            Self::Posts => "posts",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchHitKind {
    Thread,
    Post,
}

#[derive(Clone)]
pub struct SearchHit {
    pub thread_id: Uuid,
    pub post_id: Option<Uuid>,
    pub board_slug: String,
    pub thread_title: String,
    pub author_id: Uuid,
    pub body_snippet: String,
    pub created_at: DateTime<Utc>,
    pub score: i32,
    pub kind: SearchHitKind,
    pub is_saved: bool,
    pub is_hidden: bool,
}

#[derive(Debug)]
pub enum ForumActionError {
    NotFound(String),
    RateLimit(String),
    Auth(String),
    Validation(String),
    Storage(String),
}

impl ForumActionError {
    pub fn to_status_code(&self) -> (u16, &'static str) {
        match self {
            Self::NotFound(_) => (404, "not found"),
            Self::Auth(_) => (403, "forbidden"),
            Self::RateLimit(_) => (429, "too many requests"),
            Self::Validation(_) => (400, "bad request"),
            Self::Storage(_) => (500, "storage error"),
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::NotFound(msg)
            | Self::RateLimit(msg)
            | Self::Auth(msg)
            | Self::Validation(msg)
            | Self::Storage(msg) => msg.clone(),
        }
    }
}

fn self_post(author_id: Uuid, body: String, thread_id: Uuid, created_at: DateTime<Utc>) -> Post {
    Post {
        id: Uuid::new_v4(),
        thread_id,
        user_id: author_id,
        body,
        created_at,
        state: PostState::Allow,
    }
}

pub fn is_score_hidden(created_at: DateTime<Utc>) -> bool {
    (now() - created_at).num_hours() < THREAD_SCORE_HIDE_HOURS
}

fn hot_score(item: &ThreadPreview, now: DateTime<Utc>) -> f64 {
    let age = (now - item.thread.created_at).num_seconds() as f64 / 3600.0 + 1.0;
    (item.score as f64 + 1.0) / age.sqrt() * (1.0 + (item.reply_count as f64).ln_1p())
}

fn is_handle_char(value: u8) -> bool {
    value.is_ascii_alphanumeric() || value == b'_' || value == b'-'
}

fn normalize_handle(raw: &str) -> Result<String, ForumActionError> {
    let handle = raw.trim().trim_start_matches('@').to_ascii_lowercase();
    if handle.len() < 3 || handle.len() > 24 {
        return Err(ForumActionError::Validation("handle must be 3-24 characters".to_string()));
    }
    if !handle.bytes().all(is_handle_char) {
        return Err(ForumActionError::Validation(
            "handle may only use letters, numbers, underscore, and hyphen".to_string(),
        ));
    }
    Ok(handle)
}

fn mentioned_handles(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut handles = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'@' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && is_handle_char(bytes[j]) {
                j += 1;
            }
            if j > start {
                if let Ok(handle) = std::str::from_utf8(&bytes[start..j]) {
                    let candidate = handle.to_lowercase();
                    if !candidate.is_empty() {
                        handles.push(candidate);
                    }
                }
            }
            i = j;
            continue;
        }

        if bytes[i] == b'u' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            let start = i + 2;
            let mut j = start;
            while j < bytes.len() && is_handle_char(bytes[j]) {
                j += 1;
            }
            if j > start {
                if let Ok(handle) = std::str::from_utf8(&bytes[start..j]) {
                    let candidate = handle.to_lowercase();
                    if !candidate.is_empty() {
                        handles.push(candidate);
                    }
                }
            }
            i = j;
            continue;
        }

        i += 1;
    }

    handles
}

fn thread_count_for_user_in_board(store: &ForumStore, user_id: Uuid, board_slug: &str) -> usize {
    store
        .threads
        .values()
        .filter(|thread| thread.user_id == user_id && thread.board_slug == board_slug)
        .count()
}

fn truncate_plain_text(value: &str, limit: usize) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= limit {
        return trimmed.to_string();
    }

    let mut output: String = trimmed.chars().take(limit).collect();
    output.push('…');
    output
}

fn normalize_title(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch.to_ascii_lowercase() } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn reject_spam_body(value: &str) -> Result<(), ForumActionError> {
    let link_count = value
        .split_whitespace()
        .filter(|part| part.starts_with("http://") || part.starts_with("https://"))
        .count();
    if link_count > 8 {
        return Err(ForumActionError::RateLimit("too many links".to_string()));
    }
    let repeated_run = value
        .chars()
        .fold((None, 0usize, 0usize), |(last, run, max_run), ch| {
            if Some(ch) == last {
                (last, run + 1, max_run.max(run + 1))
            } else {
                (Some(ch), 1, max_run.max(1))
            }
        })
        .2;
    if repeated_run > 80 {
        return Err(ForumActionError::RateLimit("repeated text limit reached".to_string()));
    }
    Ok(())
}

pub fn normalize_thread_tags(tags: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::new();
    let mut seen = HashSet::new();
    for tag in tags {
        let slug = normalize_tag(&tag);
        if slug.is_empty() || !seen.insert(slug.clone()) {
            continue;
        }
        normalized.push(slug);
        if normalized.len() == MAX_THREAD_TAGS {
            break;
        }
    }
    normalized
}

fn normalize_tag(value: &str) -> String {
    let mut output = String::new();
    let mut last_dash = false;
    for ch in value.trim().chars() {
        if output.chars().count() >= MAX_THREAD_TAG_CHARS {
            break;
        }
        if ch.is_ascii_alphanumeric() {
            output.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if (ch.is_whitespace() || ch == '-' || ch == '_') && !output.is_empty() && !last_dash
        {
            output.push('-');
            last_dash = true;
        }
    }
    output.trim_matches('-').to_string()
}

fn query_terms(value: &str) -> Vec<String> {
    value
        .split_whitespace()
        .map(|part| part.trim().to_ascii_lowercase())
        .filter(|part| !part.is_empty())
        .collect()
}

pub(crate) struct ParsedSearchQuery {
    pub(crate) terms: Vec<String>,
    pub(crate) board_slug: Option<String>,
    pub(crate) tag: Option<String>,
    pub(crate) author_handle: Option<String>,
}

pub(crate) fn parse_search_query(value: &str) -> ParsedSearchQuery {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return ParsedSearchQuery {
            terms: Vec::new(),
            board_slug: None,
            tag: None,
            author_handle: None,
        };
    }

    let mut search_text = trimmed;
    let mut board_slug = None;
    let mut tag = None;
    let mut author_handle = None;

    if let Some((query_text, scope_text)) = trimmed.rsplit_once(':') {
        let scope_text = scope_text.trim();
        if let Some(candidate_slug) = scope_text.strip_prefix('/') {
            let candidate_slug = candidate_slug.trim().to_ascii_lowercase();
            if !candidate_slug.is_empty()
                && candidate_slug.chars().all(|ch| {
                    ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_'
                })
            {
                search_text = query_text.trim();
                board_slug = Some(candidate_slug);
            }
        }
    }

    let mut terms = Vec::new();
    for term in query_terms(search_text) {
        if let Some(raw_tag) = term.strip_prefix("tag:").or_else(|| term.strip_prefix('#')) {
            tag = normalize_thread_tags(vec![raw_tag.to_string()]).into_iter().next();
        } else if let Some(raw_author) = term.strip_prefix("author:") {
            author_handle = Some(raw_author.trim_start_matches('@').to_ascii_lowercase());
        } else {
            terms.push(term);
        }
    }

    if terms.is_empty() && (tag.is_some() || author_handle.is_some()) {
        terms.push(String::new());
    }

    ParsedSearchQuery { terms, board_slug, tag, author_handle }
}

fn text_matches_all_terms(text: &str, terms: &[String]) -> bool {
    if terms.is_empty() {
        return false;
    }

    terms.iter().all(|term| text.contains(term))
}

fn search_snippet(value: &str, limit: usize) -> String {
    truncate_plain_text(value, limit)
}

pub fn search_snippet_for_display(value: &str, limit: usize) -> String {
    search_snippet(value, limit)
}

fn default_subscription_set(boards: &[crate::models::Board]) -> HashSet<String> {
    boards.iter().take(4).map(|board| board.slug.clone()).collect()
}

fn prune_attempt_map(map: &mut HashMap<String, Vec<DateTime<Utc>>>, threshold: DateTime<Utc>) {
    map.retain(|_, attempts| {
        attempts.retain(|attempt| *attempt >= threshold);
        !attempts.is_empty()
    });
}

fn _to_u32(value: &str) -> Option<u32> {
    u32::from_str(value).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successful_legacy_password_login_upgrades_hash() {
        let state_path = scratch_state_path();
        let mut store = ForumStore { state_path: state_path.clone(), ..ForumStore::default() };
        store.users.insert(
            ROOT_USER_ID,
            ForumUserRecord {
                id: ROOT_USER_ID,
                handle: ROOT_HANDLE.to_string(),
                password_hash: legacy_password_hash(ROOT_HANDLE, ROOT_HANDLE),
                role: UserRole::Founder,
                is_banned: false,
                probation_until: None,
                created_at: now(),
                last_known_ip: None,
            },
        );

        assert_eq!(store.verify_user_credentials(ROOT_HANDLE, ROOT_HANDLE), Some(ROOT_USER_ID));

        let root = store.users.get(&ROOT_USER_ID).expect("root should exist");
        assert!(root.password_hash.starts_with("$argon2id$"));
        let _ = fs::remove_file(state_path);
    }

    #[tokio::test]
    async fn strict_local_startup_rejects_invalid_state_file() {
        let boards = vec![crate::models::Board {
            slug: "general".to_string(),
            name: "General".to_string(),
            ttl_days: 180,
            description: String::new(),
        }];
        let state_path = std::env::temp_dir().join(format!("viceden-bad-{}.json", Uuid::new_v4()));
        fs::write(&state_path, "{not json").expect("write invalid state");

        let error = match ForumStore::try_with_state_path_and_pool(&boards, &state_path, None).await
        {
            Ok(_) => panic!("invalid state should fail startup"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("state file is invalid"));
        let _ = fs::remove_file(state_path);
    }

    #[tokio::test]
    async fn strict_local_startup_rejects_unwritable_state_path() {
        let parent_file =
            std::env::temp_dir().join(format!("viceden-parent-file-{}", Uuid::new_v4()));
        fs::write(&parent_file, "not a directory").expect("write parent file");
        let state_path = parent_file.join("forum_state.json");
        let store = ForumStore { state_path, ..ForumStore::default() };

        let error = match store.try_persist_state() {
            Ok(_) => panic!("unwritable state path should fail"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("state directory creation failed"));
        let _ = fs::remove_file(parent_file);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn required_postgres_create_user_fails_closed_on_storage_error() {
        let boards = test_boards();
        let state_path = scratch_state_path();
        let mut store = ForumStore::seed_from_boards(&boards, &state_path);
        store.set_required_postgres_for_tests(broken_postgres_pool());

        let result = store.create_user("closeduser", "secret123", Some("203.0.113.10".into()));

        assert!(matches!(result, Err(ForumActionError::Storage(_))));
        assert!(store.user_id_for_handle("closeduser").is_none());
        assert_eq!(store.database_status_label(), "error");
        let _ = fs::remove_file(state_path);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn required_postgres_create_thread_fails_closed_on_storage_error() {
        let boards = test_boards();
        let state_path = scratch_state_path();
        let mut store = ForumStore::seed_from_boards(&boards, &state_path);
        store.set_required_postgres_for_tests(broken_postgres_pool());
        let starting_count = store.all_threads().len();

        let result = store.create_thread_with_checks(
            "general",
            format!("storage closed {}", Uuid::new_v4()),
            "body".to_string(),
            ROOT_USER_ID,
        );

        assert!(matches!(result, Err(ForumActionError::Storage(_))));
        assert_eq!(store.all_threads().len(), starting_count);
        assert_eq!(store.database_status_label(), "error");
        let _ = fs::remove_file(state_path);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn required_postgres_session_lookup_does_not_fallback_to_local_cache() {
        let boards = test_boards();
        let state_path = scratch_state_path();
        let mut store = ForumStore::seed_from_boards(&boards, &state_path);
        let token = "cached-session".to_string();
        store.sessions.insert(
            token.clone(),
            SessionRecord {
                user_id: ROOT_USER_ID,
                created_at: now(),
                expires_at: now() + Duration::days(1),
                csrf_token: "csrf".to_string(),
            },
        );
        store.set_required_postgres_for_tests(broken_postgres_pool());

        assert_eq!(store.get_session_user(&token), None);
        assert_eq!(store.csrf_token_for_session(&token), None);
        assert_eq!(store.database_status_label(), "error");
        let _ = fs::remove_file(state_path);
    }

    /// A throwaway state file. Stores built with `ForumStore::default()` persist to the tracked
    /// `data/forum_state.json`, so tests that mutate a store must point it somewhere disposable.
    fn scratch_state_path() -> PathBuf {
        std::env::temp_dir().join(format!("viceden-forum-test-{}.json", Uuid::new_v4()))
    }

    fn test_boards() -> Vec<crate::models::Board> {
        vec![crate::models::Board {
            slug: "general".to_string(),
            name: "General".to_string(),
            ttl_days: 180,
            description: String::new(),
        }]
    }

    fn broken_postgres_pool() -> PgPool {
        sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(std::time::Duration::from_millis(100))
            .connect_lazy("postgres://viceden:bad@127.0.0.1:1/viceden")
            .expect("lazy pool should build")
    }
}

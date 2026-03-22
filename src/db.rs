use crate::models::*;
use sqlx::PgPool;

// ─── Messages ───────────────────────────────────────────────────

pub async fn insert_message(
    pool: &PgPool,
    channel_id: ChannelId,
    author_id: UserId,
    content: &str,
) -> Result<Message, sqlx::Error> {
    sqlx::query_as::<_, Message>(
        r#"
        INSERT INTO messages (channel_id, author_id, content)
        VALUES ($1, $2, $3)
        RETURNING *
        "#,
    )
    .bind(channel_id)
    .bind(author_id)
    .bind(content)
    .fetch_one(pool)
    .await
}

pub async fn update_message(
    pool: &PgPool,
    message_id: MessageId,
    author_id: UserId,
    content: &str,
) -> Result<Option<Message>, sqlx::Error> {
    sqlx::query_as::<_, Message>(
        r#"
        UPDATE messages
        SET content = $1, edited_at = now()
        WHERE id = $2 AND author_id = $3
        RETURNING *
        "#,
    )
    .bind(content)
    .bind(message_id)
    .bind(author_id)
    .fetch_optional(pool)
    .await
}

pub async fn delete_message(
    pool: &PgPool,
    message_id: MessageId,
    author_id: UserId,
) -> Result<bool, sqlx::Error> {
    let result =
        sqlx::query("DELETE FROM messages WHERE id = $1 AND author_id = $2")
            .bind(message_id)
            .bind(author_id)
            .execute(pool)
            .await?;
    Ok(result.rows_affected() > 0)
}

// ─── Channel members ────────────────────────────────────────────

pub async fn get_channel_member_ids(
    pool: &PgPool,
    channel_id: ChannelId,
) -> Result<Vec<UserId>, sqlx::Error> {
    let rows = sqlx::query_scalar::<_, UserId>(
        r#"
        SELECT gm.user_id
        FROM channels c
        JOIN guild_members gm ON gm.guild_id = c.guild_id
        WHERE c.id = $1
        "#,
    )
    .bind(channel_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Get the guild that a channel belongs to.
pub async fn get_channel_guild_id(
    pool: &PgPool,
    channel_id: ChannelId,
) -> Result<Option<GuildId>, sqlx::Error> {
    sqlx::query_scalar::<_, GuildId>("SELECT guild_id FROM channels WHERE id = $1")
        .bind(channel_id)
        .fetch_optional(pool)
        .await
}

// ─── Guild members ──────────────────────────────────────────────

pub async fn get_guild_member_ids(
    pool: &PgPool,
    guild_id: GuildId,
) -> Result<Vec<UserId>, sqlx::Error> {
    sqlx::query_scalar::<_, UserId>(
        "SELECT user_id FROM guild_members WHERE guild_id = $1",
    )
    .bind(guild_id)
    .fetch_all(pool)
    .await
}

/// Get all guild IDs a user belongs to.
pub async fn get_user_guild_ids(
    pool: &PgPool,
    user_id: UserId,
) -> Result<Vec<GuildId>, sqlx::Error> {
    sqlx::query_scalar::<_, GuildId>(
        "SELECT guild_id FROM guild_members WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

/// Get all user IDs who share at least one guild with the given user.
pub async fn get_colocated_user_ids(
    pool: &PgPool,
    user_id: UserId,
) -> Result<Vec<UserId>, sqlx::Error> {
    sqlx::query_scalar::<_, UserId>(
        r#"
        SELECT DISTINCT gm2.user_id
        FROM guild_members gm1
        JOIN guild_members gm2 ON gm1.guild_id = gm2.guild_id
        WHERE gm1.user_id = $1 AND gm2.user_id != $1
        "#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

// ─── READY payload builders ─────────────────────────────────────

pub async fn get_user_guilds_info(
    pool: &PgPool,
    user_id: UserId,
) -> Result<Vec<GuildInfo>, sqlx::Error> {
    // Fetch guilds
    let guilds = sqlx::query_as::<_, Guild>(
        r#"
        SELECT g.*
        FROM guilds g
        JOIN guild_members gm ON gm.guild_id = g.id
        WHERE gm.user_id = $1
        "#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    let mut guild_infos = Vec::with_capacity(guilds.len());
    for guild in guilds {
        let channels = sqlx::query_as::<_, Channel>(
            "SELECT * FROM channels WHERE guild_id = $1 ORDER BY created_at",
        )
        .bind(guild.id)
        .fetch_all(pool)
        .await?;

        let member_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM guild_members WHERE guild_id = $1",
        )
        .bind(guild.id)
        .fetch_one(pool)
        .await?;

        guild_infos.push(GuildInfo {
            id: guild.id,
            name: guild.name,
            channels: channels
                .into_iter()
                .map(|c| ChannelInfo {
                    id: c.id,
                    name: c.name,
                })
                .collect(),
            member_count,
        });
    }

    Ok(guild_infos)
}

// ─── Read states ────────────────────────────────────────────────

pub async fn get_user_read_states(
    pool: &PgPool,
    user_id: UserId,
) -> Result<Vec<ReadState>, sqlx::Error> {
    sqlx::query_as::<_, ReadState>(
        r#"
        SELECT channel_id, last_read_message_id
        FROM read_states
        WHERE user_id = $1
        "#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

pub async fn upsert_read_states_batch(
    pool: &PgPool,
    updates: &[(UserId, ChannelId, MessageId)],
) -> Result<(), sqlx::Error> {
    if updates.is_empty() {
        return Ok(());
    }

    // Build a bulk upsert using UNNEST for efficiency
    let user_ids: Vec<UserId> = updates.iter().map(|(u, _, _)| *u).collect();
    let channel_ids: Vec<ChannelId> = updates.iter().map(|(_, c, _)| *c).collect();
    let message_ids: Vec<MessageId> = updates.iter().map(|(_, _, m)| *m).collect();

    sqlx::query(
        r#"
        INSERT INTO read_states (user_id, channel_id, last_read_message_id, updated_at)
        SELECT * FROM UNNEST($1::uuid[], $2::uuid[], $3::uuid[], ARRAY[now()])
        ON CONFLICT (user_id, channel_id) DO UPDATE
        SET last_read_message_id = EXCLUDED.last_read_message_id,
            updated_at = now()
        WHERE read_states.last_read_message_id < EXCLUDED.last_read_message_id
        "#,
    )
    .bind(&user_ids)
    .bind(&channel_ids)
    .bind(&message_ids)
    .execute(pool)
    .await?;

    Ok(())
}

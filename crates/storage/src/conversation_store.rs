use super::*;
use rusqlite::Row;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug)]
pub struct TranscriptSession<'a> {
    pub harness: Backend,
    pub path: &'a Path,
    pub project: &'a Path,
    pub profile_id: Option<&'a str>,
}

#[derive(Clone, Debug)]
pub struct StoredTranscript {
    pub messages: Vec<Value>,
    pub complete: bool,
    pub complete_from_start: bool,
    pub total: usize,
    pub revision: i64,
    pub stamp: TranscriptStamp,
    pub metadata: Value,
}

type Error = Box<dyn std::error::Error>;
type Result<T, E = Error> = std::result::Result<T, E>;

impl SharedStateStore {
    pub(super) fn with_transcript_connection<T>(
        &self,
        operation: impl FnOnce(&mut StateStore) -> Result<T, String>,
    ) -> Result<T, String> {
        let Some(database) = &self.1 else {
            return self.with(operation);
        };
        let cached = database
            .idle_connection
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        let connection = match cached {
            Some(connection) => connection,
            None => {
                let connection = Connection::open_with_flags(
                    &database.path,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
                        | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
                )
                .map_err(|e| format!("open transcript database: {e}"))?;
                connection
                    .busy_timeout(DATABASE_BUSY_TIMEOUT)
                    .map_err(|e| e.to_string())?;
                connection
                    .pragma_update(None, "foreign_keys", true)
                    .map_err(|e| e.to_string())?;
                connection
            }
        };
        let mut store = StateStore {
            connection,
            image_directory: database.images.clone(),
        };
        let result = operation(&mut store);
        if result.is_ok() && store.connection.is_autocommit() {
            let mut idle = database
                .idle_connection
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if idle.is_none() {
                *idle = Some(store.connection);
            }
        }
        result
    }
}

impl TranscriptSession<'_> {
    pub(super) fn find(&self, db: &Connection) -> rusqlite::Result<Option<i64>> {
        db.query_row(
            "SELECT s.id FROM sessions s JOIN projects p ON p.id=s.project_id
             WHERE s.harness=?1 AND s.locator=?2 AND p.path=?3 AND s.profile_id IS ?4",
            params![
                self.harness.as_str(),
                sessions::normalize_session_path(self.path).to_string_lossy(),
                sessions::normalize_session_path(self.project).to_string_lossy(),
                self.profile_id
            ],
            |row| row.get(0),
        )
        .optional()
    }

    fn ensure(&self, tx: &Transaction<'_>) -> Result<i64> {
        if let Some(id) = self.find(tx)? {
            return Ok(id);
        }
        let now = u64_to_i64(now_ms());
        let project = ensure_project(tx, self.project, now)?;
        let locator = sessions::normalize_session_path(self.path);
        tx.execute(
            "INSERT INTO sessions(harness,locator,project_id,profile_id,modified_ms,created_ms)
             VALUES(?1,?2,?3,?4,?5,?5)",
            params![
                self.harness.as_str(),
                locator.to_string_lossy(),
                project,
                self.profile_id,
                now
            ],
        )?;
        Ok(tx.last_insert_rowid())
    }
}

fn text<'a>(message: &'a Value, field: &str) -> Option<&'a str> {
    message.get(field)?.as_str().filter(|id| !id.is_empty())
}

fn native_key(message: &Value) -> Option<String> {
    let role = text(message, "role")?;
    let turn = text(message, "nativeTurnId");
    if role == "toolResult"
        && let Some(id) = text(message, "toolCallId")
    {
        return Some(json!(["result", id]).to_string());
    }
    if role == "assistant"
        && let [block] = message.get("content")?.as_array()?.as_slice()
        && text(block, "type") == Some("toolCall")
    {
        return Some(json!(["call", text(block, "id")?]).to_string());
    }
    if let Some(id) = text(message, "nativeMessageId").or_else(|| text(message, "id")) {
        return Some(json!([role, turn, id]).to_string());
    }
    None
}

fn digest(message: &Value) -> [u8; 32] {
    Sha256::digest(serde_json::to_vec(message).expect("JSON value is serializable")).into()
}

fn submission_id(message: &Value) -> Option<&str> {
    (text(message, "role") == Some("user"))
        .then(|| text(message, "submissionId"))
        .flatten()
}

// Store the positions of externalized image blocks separately. Adapter-supplied JSON
// can never turn a path or a lookalike application field into a file read.
#[derive(Serialize, Deserialize)]
struct BlockImage {
    block: usize,
    image: crate::images::StoredImage,
}

pub(super) fn viewport_page_start(
    db: &Connection,
    id: i64,
    start: usize,
) -> rusqlite::Result<usize> {
    const EXTRA_MESSAGES: usize = 1024;
    const EXTRA_BYTES: i64 = 8 * 1024 * 1024;
    let earliest = start.saturating_sub(EXTRA_MESSAGES);
    let anchor = db
        .query_row(
            "SELECT ordinal FROM transcript_messages WHERE session_id=?1 AND user_message=1
            AND ordinal BETWEEN ?2 AND ?3 ORDER BY ordinal DESC LIMIT 1",
            params![id, earliest, start],
            |r| r.get::<_, usize>(0),
        )
        .optional()?;
    let candidate = anchor.unwrap_or(if earliest == 0 { 0 } else { start });
    if candidate == start {
        return Ok(start);
    }
    let bytes: i64 = db.query_row(
        "SELECT coalesce(sum(octet_length(body)),0) FROM transcript_messages WHERE session_id=?1
            AND ordinal>=?2 AND ordinal<?3",
        params![id, candidate, start],
        |r| r.get(0),
    )?;
    Ok(if bytes <= EXTRA_BYTES {
        candidate
    } else {
        start
    })
}

pub(super) const TRANSCRIPT_SELECT: &str = "SELECT m.body,m.images,m.submission_id,COALESCE(o.images_json,json_extract(e.body,'$.images')),o.display_message,o.invocation,m.ordinal,m.source_digest
             FROM transcript_messages m
             LEFT JOIN session_events e ON e.session_id=m.session_id AND e.seq=(
                 SELECT min(r.seq) FROM session_events r
                 WHERE m.submission_id IS NOT NULL AND r.session_id=m.session_id
                   AND json_extract(r.body,'$.type')='accepted_prompt'
                   AND CAST(json_extract(r.body,'$.submissionId') AS TEXT)=m.submission_id
                 HAVING count(*)=1)
             LEFT JOIN outbox o ON o.session_id=m.session_id AND o.id=json_extract(e.body,'$.outboxId')";

impl StateStore {
    fn encode_message(&self, message: &Value) -> Result<(String, String)> {
        let mut references = Vec::new();
        let mut copied = None;
        for (index, block) in message
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            if text(block, "type") != Some("image") {
                continue;
            }
            let (Some(data), Some(mime)) = (text(block, "data"), text(block, "mimeType")) else {
                continue;
            };
            let image = PromptImage::new(data.to_owned(), mime.to_owned());
            references.push(BlockImage {
                block: index,
                image: self.encode_prompt_image(&image)?,
            });
            let body = copied.get_or_insert_with(|| message.clone());
            body["content"][index]
                .as_object_mut()
                .expect("image block")
                .remove("data");
        }
        Ok((
            serde_json::to_string(copied.as_ref().unwrap_or(message))?,
            serde_json::to_string(&references)?,
        ))
    }

    pub(super) fn decode_message(&self, row: &Row<'_>) -> Result<Value> {
        self.decode_message_with_images(row, |image| Ok(image.into_inline()?))
    }

    pub(super) fn decode_message_with_images(
        &self,
        row: &Row<'_>,
        mut inline: impl FnMut(PromptImage) -> Result<PromptImage>,
    ) -> Result<Value> {
        let mut message: Value = serde_json::from_str(row.get_ref(0)?.as_str()?)?;
        let references: Vec<BlockImage> = serde_json::from_str(row.get_ref(1)?.as_str()?)?;
        let originals: Vec<crate::images::StoredImage> = match row.get_ref(3)?.as_str_or_null()? {
            Some(images) => serde_json::from_str(images)?,
            None => Vec::new(),
        };
        let mut originals = originals.into_iter();
        if let Some(submission) = row.get::<_, Option<String>>(2)? {
            message["submissionId"] = submission.into();
        }
        if originals.len() > 0
            && let Some(value) = message.get_mut("content").filter(|v| v.is_string())
        {
            *value = json!([{"type":"text","text":value.take()}]);
        }
        if let Some(content) = message.get_mut("content").and_then(Value::as_array_mut) {
            let mut references = references.into_iter().peekable();
            for (index, block) in content.iter_mut().enumerate() {
                if text(block, "type") != Some("image") {
                    continue;
                }
                let archived = references.next_if(|r| r.block == index);
                if let Some(reference) = originals.next().or_else(|| archived.map(|r| r.image)) {
                    let image = inline(self.decode_prompt_image(reference)?)?;
                    block["data"] = image.data.into();
                    block["mimeType"] = image.mime_type.into();
                }
            }
            if references.next().is_some() {
                return Err("invalid stored image block".into());
            }
            for reference in originals {
                content.push(serde_json::to_value(inline(
                    self.decode_prompt_image(reference)?,
                )?)?);
            }
        }
        if let (Some(display), Some(invocation)) = (
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<String>>(5)?,
        ) {
            message["farcasterUserInvocation"] = display.into();
            message["farcasterInvocationResolution"] = invocation.into();
        }
        Ok(message)
    }

    pub(super) fn restore_links(
        &self,
        session: TranscriptSession<'_>,
        messages: &mut Arc<Vec<Value>>,
    ) -> Result<()> {
        let tx = self.connection.unchecked_transaction()?;
        let Some(id) = session.find(&tx)? else {
            return Ok(());
        };
        let mut statement = tx.prepare_cached(&format!(
            "{TRANSCRIPT_SELECT}
             WHERE m.session_id=?1 AND (m.submission_id IS NOT NULL OR m.inherited_fields=1)
             ORDER BY m.ordinal"
        ))?;
        let mut rows = statement.query([id])?;
        let mut changes = Vec::new();
        while let Some(row) = rows.next()? {
            let index: usize = row.get(6)?;
            let Some(original) = messages.get(index) else {
                continue;
            };
            if row.get_ref(7)?.as_blob()? != digest(original) {
                continue;
            }
            let linked = self.decode_message(row)?;
            if linked != *original {
                changes.push((index, linked));
            }
        }
        if !changes.is_empty() {
            let messages = Arc::make_mut(messages);
            for (index, linked) in changes {
                messages[index] = linked;
            }
        }
        Ok(())
    }

    pub(super) fn read_conversation(
        &self,
        session: TranscriptSession<'_>,
        limit: Option<usize>,
        align_to_turn: bool,
    ) -> Result<Option<StoredTranscript>, String> {
        self.read_conversation_inner(session, limit, align_to_turn)
            .map_err(|e| format!("read stored transcript: {e}"))
    }
    fn read_conversation_inner(
        &self,
        session: TranscriptSession<'_>,
        limit: Option<usize>,
        align_to_turn: bool,
    ) -> Result<Option<StoredTranscript>> {
        let tx = self.connection.unchecked_transaction()?;
        let Some(id) = session.find(&tx)? else {
            return Ok(None);
        };
        self.read_conversation_transaction(&tx, id, limit, align_to_turn)
    }

    pub(super) fn read_conversation_transaction(
        &self,
        tx: &Transaction<'_>,
        id: i64,
        limit: Option<usize>,
        align_to_turn: bool,
    ) -> Result<Option<StoredTranscript>> {
        let select = format!(
            "SELECT revision,complete,item_count,metadata,({}) FROM transcript_documents WHERE session_id=?1",
            transcript_viewport::PRESENTATION_STAMP
        );
        let Some((revision, full, total, metadata, prompt_event_seq)) = tx
            .query_row(&select, [id], |r| {
                Ok((
                    r.get(0)?,
                    r.get::<_, bool>(1)?,
                    r.get::<_, usize>(2)?,
                    r.get::<_, String>(3)?,
                    r.get(4)?,
                ))
            })
            .optional()?
        else {
            return Ok(None);
        };
        let mut start = total.saturating_sub(limit.unwrap_or(total));
        if align_to_turn && start > 0 {
            start = viewport_page_start(&tx, id, start)?;
        }
        let count = total - start;
        let mut statement = tx.prepare_cached(&format!(
            "{TRANSCRIPT_SELECT} WHERE m.session_id=?1 AND m.ordinal>=?2 ORDER BY m.ordinal"
        ))?;
        let mut rows = statement.query(params![id, start])?;
        let mut messages = Vec::with_capacity(count);
        while let Some(row) = rows.next()? {
            messages.push(self.decode_message(row)?);
        }
        Ok(Some(StoredTranscript {
            complete: full && count == total,
            complete_from_start: full,
            revision,
            stamp: TranscriptStamp {
                session_id: id,
                revision,
                prompt_event_seq,
            },
            total,
            messages,
            metadata: serde_json::from_str(&metadata)?,
        }))
    }
    pub(super) fn replace_conversation(
        &self,
        session: TranscriptSession<'_>,
        expected: i64,
        messages: &[Value],
        metadata: &Value,
    ) -> Result<Option<TranscriptStamp>, String> {
        let applied = self
            .replace_conversation_inner(session, expected, messages, metadata)
            .map_err(|e| format!("import transcript: {e}"))?;
        if applied.is_some() {
            self.refresh_viewport_for_session(session);
        }
        Ok(applied)
    }
    fn replace_conversation_inner(
        &self,
        session: TranscriptSession<'_>,
        expected: i64,
        messages: &[Value],
        metadata: &Value,
    ) -> Result<Option<TranscriptStamp>> {
        let read = self.connection.unchecked_transaction()?;
        let id = session.find(&read)?;
        let info = read
            .query_row(
                "SELECT revision,complete,metadata FROM transcript_documents WHERE session_id=?1",
                [id],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, bool>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        if info.as_ref().map_or(0, |header| header.0) != expected {
            return Ok(None);
        }
        let fingerprints = {
            let mut stmt=read.prepare("SELECT source_digest FROM transcript_messages WHERE session_id=?1 ORDER BY ordinal")?;
            stmt.query_map([id], |r| r.get::<_, Vec<u8>>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        read.commit()?;
        let mut prefix = 0;
        let mut suffix = Vec::new();
        for (i, message) in messages.iter().enumerate() {
            let hash = digest(message);
            if i == prefix
                && fingerprints
                    .get(i)
                    .is_some_and(|old| old.as_slice() == hash)
            {
                prefix += 1;
            } else {
                let (body, images) = self.encode_message(message)?;
                suffix.push((hash, body, images));
            }
        }
        let metadata = serde_json::to_string(metadata)?;
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let id = session.ensure(&tx)?;
        let revision = tx
            .query_row(
                "SELECT revision FROM transcript_documents WHERE session_id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0);
        if expected != revision {
            return Ok(None);
        }
        if prefix == messages.len()
            && prefix == fingerprints.len()
            && info
                .as_ref()
                .is_some_and(|(_, complete, previous)| *complete && previous == &metadata)
        {
            return Ok(self.transcript_stamp_for_id(id)?);
        }
        let mut existing = HashMap::new();
        {
            let mut stmt = tx.prepare(
                "SELECT item_key,native_key,submission_id,
                    CASE WHEN native_key IS NOT NULL OR submission_id IS NOT NULL THEN
                      json_object('nativeMessageId', json_extract(body,'$.nativeMessageId'),
                                  'nativeTurnId',json_extract(body,'$.nativeTurnId'),
                                  'farcasterToolCall',json_extract(body,'$.farcasterToolCall'),
                                  'details',json_extract(CASE WHEN json_extract(body,'$.role')='toolResult' THEN body ELSE '{}' END,'$.details')) END
                 FROM transcript_messages WHERE session_id=?1 AND ordinal>=?2",
            )?;
            let mut rows = stmt.query(params![id, prefix])?;
            while let Some(row) = rows.next()? {
                let key: String = row.get(0)?;
                let native: Option<String> = row.get(1)?;
                let submission: Option<String> = row.get(2)?;
                let source_fields: Option<String> = row.get(3)?;
                for identity in [
                    native,
                    submission
                        .as_ref()
                        .map(|s| json!(["submission", s]).to_string()),
                ]
                .into_iter()
                .flatten()
                {
                    existing
                        .entry(identity)
                        .and_modify(|entry| *entry = None)
                        .or_insert_with(|| {
                            Some((key.clone(), submission.clone(), source_fields.clone()))
                        });
                }
            }
        }
        tx.execute("INSERT INTO transcript_documents(session_id,revision,complete,item_count,metadata,updated_ms) VALUES(?1,?2,1,?3,?4,?5)
                   ON CONFLICT(session_id) DO UPDATE SET revision=excluded.revision,complete=1,item_count=excluded.item_count,metadata=excluded.metadata,updated_ms=excluded.updated_ms",
                   params![id,revision+1,messages.len(),metadata,u64_to_i64(now_ms())])?;
        tx.execute(
            "DELETE FROM transcript_messages WHERE session_id=?1 AND ordinal>=?2",
            params![id, prefix],
        )?;
        let mut native_counts = HashMap::new();
        let mut submission_counts = HashMap::new();
        if !existing.is_empty() {
            for message in messages {
                if let Some(key) = native_key(message) {
                    *native_counts.entry(key).or_insert(0) += 1;
                }
                if let Some(key) = submission_id(message) {
                    *submission_counts.entry(key).or_insert(0) += 1;
                }
            }
        }
        let mut used_keys = HashSet::new();
        let mut statement=tx.prepare_cached("INSERT INTO transcript_messages(session_id,item_key,ordinal,native_key,submission_id,body,images,source_digest,user_message,inherited_fields) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)")?;
        for ((ordinal, message), (hash, body, images)) in
            messages.iter().enumerate().skip(prefix).zip(suffix)
        {
            let native = native_key(message);
            let submission = submission_id(message);
            let old = native
                .as_ref()
                .filter(|k| native_counts.get(*k) == Some(&1))
                .and_then(|s| existing.get(s))
                .or_else(|| {
                    submission
                        .filter(|s| submission_counts.get(s) == Some(&1))
                        .and_then(|s| existing.get(&json!(["submission", s]).to_string()))
                })
                .and_then(Option::as_ref);
            let old = old.filter(|(key, sub, _)| {
                !used_keys.contains(key)
                    && !matches!((submission,sub.as_deref()),(Some(a),Some(b)) if a!=b)
            });
            let key = old
                .map(|x| x.0.clone())
                .unwrap_or_else(|| format!("import/{}/{ordinal}", revision + 1));
            let submission = submission.or_else(|| old.and_then(|x| x.1.as_deref()));
            let mut source = native;
            let mut inherited = false;
            let body = if let Some(fields) = old.and_then(|x| x.2.as_deref()) {
                let fields: serde_json::Map<String, Value> = serde_json::from_str(fields)?;
                let mut merged = None;
                for (field, value) in fields {
                    if !value.is_null() && message.get(&field).is_none() {
                        let stored: &mut Value = match &mut merged {
                            Some(body) => body,
                            None => merged.insert(serde_json::from_str(&body)?),
                        };
                        stored[field] = value;
                    }
                }
                if let Some(merged) = merged {
                    inherited = true;
                    source = native_key(&merged);
                    serde_json::to_string(&merged)?
                } else {
                    body
                }
            } else {
                body
            };
            statement.execute(params![
                id,
                key,
                ordinal,
                source,
                submission,
                body,
                images,
                hash.as_slice(),
                text(message, "role") == Some("user"),
                inherited
            ])?;
            used_keys.insert(key);
        }
        drop(statement);
        let stamp = self.transcript_stamp_for_id(id)?;
        tx.commit()?;
        Ok(stamp)
    }
    pub(super) fn write_conversation_items(
        &self,
        session: TranscriptSession<'_>,
        items: &[(String, Value)],
        complete: bool,
    ) -> Result<(), String> {
        self.write_conversation_items_inner(session, items, complete)
            .map_err(|e| format!("save transcript items: {e}"))?;
        if !items.is_empty() {
            self.schedule_session_viewport(session);
        }
        Ok(())
    }
    fn write_conversation_items_inner(
        &self,
        session: TranscriptSession<'_>,
        items: &[(String, Value)],
        complete: bool,
    ) -> Result<()> {
        if items.is_empty() {
            return Ok(());
        }
        let encoded = items
            .iter()
            .map(|(_, message)| {
                let (body, images) = self.encode_message(message)?;
                Ok((digest(message), body, images))
            })
            .collect::<Result<Vec<_>>>()?;
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let id = session
            .find(&tx)?
            .ok_or("session has no matching saved identity")?;
        tx.execute(
            "INSERT OR IGNORE INTO transcript_documents(session_id) VALUES(?1)",
            [id],
        )?;
        let mut count: usize = tx.query_row(
            "SELECT item_count FROM transcript_documents WHERE session_id=?1",
            [id],
            |r| r.get(0),
        )?;
        for ((key, message), (hash, body, images)) in items.iter().zip(encoded) {
            if key.is_empty() {
                return Err("empty live transcript key".into());
            }
            let native = native_key(message);
            let submission = submission_id(message);
            let local_key = format!("live/{key}");
            // Never merge solely on a shared locator, timestamp or message text.
            let old = tx
                .query_row(
                    "SELECT item_key FROM transcript_messages WHERE session_id=?1 AND item_key=?2",
                    params![id, local_key],
                    |r| r.get::<_, String>(0),
                )
                .optional()?;
            let found = if old.is_some() {
                old
            } else {
                tx.query_row("SELECT min(item_key) FROM (
                    SELECT item_key FROM transcript_messages WHERE session_id=?1 AND native_key=?2
                     AND (?3 IS NULL OR submission_id IS NULL OR submission_id=?3)
                    UNION SELECT item_key FROM transcript_messages WHERE session_id=?1 AND submission_id=?3
                    ) HAVING count(*)=1",
                    params![id,native,submission],|r|r.get::<_,String>(0)).optional()?
            };
            if let Some(stored_key) = found {
                tx.execute("UPDATE transcript_messages SET body=?3,images=?4,native_key=COALESCE(?5,native_key),submission_id=COALESCE(?6,submission_id),source_digest=?7,user_message=?8,inherited_fields=0 WHERE session_id=?1 AND item_key=?2",
                    params![id,stored_key,body,images,native,submission,hash.as_slice(),text(message,"role")==Some("user")])?;
            } else {
                tx.execute("INSERT INTO transcript_messages(session_id,item_key,ordinal,body,images,native_key,submission_id,source_digest,user_message) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                    params![id,local_key,count,body,images,native,submission,hash.as_slice(),text(message,"role")==Some("user")])?;
                count += 1;
            }
        }
        tx.execute("UPDATE transcript_documents SET revision=revision+1,complete=MAX(complete,?2),item_count=?3,metadata='{}',updated_ms=?4 WHERE session_id=?1",params![id,complete,count,u64_to_i64(now_ms())])?;
        tx.commit()?;
        Ok(())
    }
}

pub(super) fn merge(tx: &Transaction<'_>, keep: i64, other: i64) -> rusqlite::Result<()> {
    let header = |id| {
        tx.query_row(
            "SELECT complete,updated_ms,revision FROM transcript_documents WHERE session_id=?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, bool>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()
    };
    let Some(other_header) = header(other)? else {
        return Ok(());
    };
    let Some(keep_header) = header(keep)? else {
        tx.execute(
            "UPDATE transcript_documents SET session_id=?1,revision=revision+1 WHERE session_id=?2",
            params![keep, other],
        )?;
        return Ok(());
    };
    let use_other_as_base = match (keep_header.0, other_header.0) {
        (false, true) => true,
        (true, false) => false,
        (true, true) => other_header.1 > keep_header.1,
        (false, false) => other_header.1 < keep_header.1,
    };
    let (base, extra, base_header, extra_header) = if use_other_as_base {
        (other, keep, other_header, keep_header)
    } else {
        (keep, other, keep_header, other_header)
    };
    let append = !extra_header.0 && extra_header.1 >= base_header.1;
    tx.execute(
        "UPDATE transcript_messages AS m SET submission_id=(
            SELECT min(e.submission_id) FROM transcript_messages e
             WHERE e.session_id=?2 AND e.native_key=m.native_key HAVING count(*)=1)
          WHERE m.session_id=?1 AND m.submission_id IS NULL AND m.native_key IS NOT NULL",
        params![base, extra],
    )?;
    let appended = if append {
        tx.execute(
            "INSERT INTO transcript_messages(session_id,item_key,ordinal,native_key,submission_id,body,images,source_digest,user_message,inherited_fields)
             SELECT ?1, 'merged/' || ?2 || '/' || e.item_key,
                 (SELECT item_count FROM transcript_documents WHERE session_id=?1)
                   +row_number() OVER (ORDER BY e.ordinal)-1,e.native_key,e.submission_id,e.body,e.images,e.source_digest,e.user_message,e.inherited_fields
             FROM transcript_messages e WHERE e.session_id=?2 AND NOT EXISTS
              (SELECT 1 FROM transcript_messages b WHERE b.session_id=?1 AND
               (b.native_key=e.native_key OR b.submission_id=e.submission_id OR
               (b.item_key=e.item_key AND e.item_key LIKE 'live/%')))",
            params![base,extra],
        )?
    } else {
        0
    };
    tx.execute(
        "DELETE FROM transcript_documents WHERE session_id=?1",
        [extra],
    )?;
    tx.execute("UPDATE transcript_documents SET revision=?3,metadata=CASE WHEN ?4>0 THEN '{}' ELSE metadata END,item_count=(SELECT count(*) FROM transcript_messages WHERE session_id=?2),session_id=?1 WHERE session_id=?2",
        params![keep,base,keep_header.2.max(other_header.2)+1,appended])?;
    Ok(())
}

#[cfg(test)]
#[path = "conversation_store_tests.rs"]
mod tests;

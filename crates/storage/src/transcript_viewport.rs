use super::*;
use crate::conversation_store::{TRANSCRIPT_SELECT, viewport_page_start};
use farcaster_conversation::{ConversationState, TranscriptItem, TranscriptKind};
use std::{collections::HashMap, io, sync::LazyLock};

use farcaster_conversation::markdown_structure::{MarkdownIndex, MarkdownStructure};

type Error = Box<dyn std::error::Error>;
const TAIL_MESSAGES: usize = 64;
const MAX_INPUT_BYTES: i64 = 8 * 1024 * 1024;
const MAX_VIEWPORT_BYTES: usize = 1024 * 1024;
const MAX_VIEWPORT_ITEMS: usize = 1024;

pub struct StoredViewport {
    pub markdown: MarkdownIndex,
    pub content: ConversationState,
    pub coverage: agents::HistoryCoverage,
    pub revision: i64,
    pub stamp: TranscriptStamp,
    /// These items contain every locally saved row and all expanded tool details.
    pub full_document: bool,
}

// The partial (session_id,seq) index excludes metrics, archive, and other events
// that cannot affect the transcript. Changes to receipts invalidate the page.
pub(super) const PRESENTATION_STAMP: &str =
    "SELECT coalesce(max(seq),0) FROM session_events WHERE session_id=?1
     AND json_extract(body,'$.type') IN ('accepted_prompt','prompt_presentation','prompt_delivery_receipt','prompt_delivery_resolution')";

#[derive(Default)]
struct Preparation {
    backfill: bool,
    sessions: HashSet<i64>,
}

static QUEUES: LazyLock<Mutex<HashMap<PathBuf, Preparation>>> = LazyLock::new(Mutex::default);
static TRACE: LazyLock<bool> = LazyLock::new(|| {
    matches!(
        std::env::var("FARCASTER_PERF_TRACE").as_deref(),
        Ok("1" | "true" | "yes")
    )
});

impl StateStore {
    pub(super) fn read_display(
        &self,
        session: TranscriptSession<'_>,
        full_document: bool,
    ) -> Result<Option<StoredViewport>, Error> {
        let tx = self.connection.unchecked_transaction()?;
        let Some(id) = session.find(&tx)? else {
            return Ok(None);
        };
        if !full_document {
            match self.read_viewport(&tx, id, session.project) {
                Ok(Some(page)) => return Ok(Some(page)),
                Ok(None) => {}
                Err(error) => log::warn!("Decode saved transcript viewport: {error}"),
            }
        }
        let Some(document) = self.read_conversation_transaction(&tx, id, None, false)? else {
            return Ok(None);
        };
        let mut messages = Arc::new(document.messages);
        self.prompt_overlay_for_id(Some(id))?.apply(&mut messages);
        let mut content = ConversationState::default();
        content.replace_history(&messages);
        Ok(Some(StoredViewport {
            content,
            coverage: coverage(document.complete),
            revision: document.revision,
            stamp: document.stamp,
            full_document: true,
            markdown: MarkdownIndex::new(),
        }))
    }

    fn read_viewport(
        &self,
        tx: &Transaction<'_>,
        id: i64,
        project: &Path,
    ) -> Result<Option<StoredViewport>, Error> {
        let sql_started = TRACE.then(Instant::now);
        let select = format!(
            "SELECT v.revision,v.from_beginning,v.items,v.event_seq FROM transcript_documents d
             LEFT JOIN transcript_viewports v ON v.session_id=d.session_id AND v.revision=d.revision
               AND v.event_seq=({PRESENTATION_STAMP})
             WHERE d.session_id=?1"
        );
        let page = tx
            .query_row(&select, [id], |r| {
                Ok((
                    r.get::<_, Option<i64>>(0)?,
                    r.get::<_, Option<bool>>(1)?,
                    r.get::<_, Option<Vec<u8>>>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                ))
            })
            .optional()?;
        if let Some(start) = sql_started {
            let bytes = page
                .as_ref()
                .and_then(|row| row.2.as_ref())
                .map_or(0, Vec::len);
            log::info!(
                "PERF operation=transcript.viewport.sql elapsed_ms={:.3} bytes={bytes}",
                start.elapsed().as_secs_f64() * 1000.0
            );
        }
        let (revision, mut complete, bytes, prompt_event_seq) = match page {
            Some((Some(revision), Some(complete), Some(bytes), Some(seq))) => {
                (revision, complete, bytes, seq)
            }
            Some((None, _, _, _)) => {
                self.queue_viewport_preparation(Some(id));
                return Ok(None);
            }
            _ => return Ok(None),
        };
        let decode_started = TRACE.then(Instant::now);
        let (content, markdown) = match self.decode_viewport(&bytes) {
            Ok(decoded) => decoded,
            Err(error) => {
                // A file from the expanded prefix may have disappeared since preparation.
                let total: usize = tx.query_row(
                    "SELECT item_count FROM transcript_documents WHERE session_id=?1",
                    [id],
                    |row| row.get(0),
                )?;
                if total <= TAIL_MESSAGES || !self.can_display_tail_for_id(Some(id))? {
                    return Err(error);
                }
                let Some(tail) = self.prepare_page(tx, id, total - TAIL_MESSAGES, project)? else {
                    return Err(error);
                };
                complete = false;
                self.decode_viewport(&tail)?
            }
        };
        if let Some(start) = decode_started {
            log::info!(
                "PERF operation=transcript.viewport.decode elapsed_ms={:.3} items={}",
                start.elapsed().as_secs_f64() * 1000.0,
                content.items.len()
            );
        }
        Ok(Some(StoredViewport {
            revision,
            content,
            coverage: coverage(complete),
            stamp: TranscriptStamp {
                session_id: id,
                revision,
                prompt_event_seq,
            },
            full_document: false,
            markdown,
        }))
    }

    pub(super) fn refresh_viewport_for_session(&self, session: TranscriptSession<'_>) {
        let result = (|| -> Result<(), Error> {
            if let Some(id) = session.find(&self.connection)? {
                self.prepare_viewport(id)?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            log::warn!("Prepare transcript viewport: {error}");
        }
    }

    /// A stream flush can change one block several times within a turn. The
    /// reader falls back to the current document while its viewport catches up.
    pub(super) fn schedule_session_viewport(&self, session: TranscriptSession<'_>) {
        if self.connection.path().is_none_or(str::is_empty) {
            self.refresh_viewport_for_session(session);
            return;
        }
        match session.find(&self.connection) {
            Ok(Some(id)) => self.queue_viewport_preparation(Some(id)),
            Ok(None) => {}
            Err(error) => log::warn!("Queue transcript viewport: {error}"),
        }
    }

    fn prepare_viewport(&self, id: i64) -> Result<(), Error> {
        let tx = self.connection.unchecked_transaction()?;
        let select = format!("SELECT d.revision,d.complete,d.item_count,p.path,
             ({PRESENTATION_STAMP})
             FROM transcript_documents d JOIN sessions s ON s.id=d.session_id JOIN projects p ON p.id=s.project_id
             WHERE d.session_id=?1");
        let head = tx
            .query_row(&select, [id], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, bool>(1)?,
                    r.get::<_, usize>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            })
            .optional()?;
        let Some((revision, complete, total, project, event_seq)) = head else {
            return Ok(());
        };
        let current = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM transcript_viewports WHERE session_id=?1 AND revision=?2 AND event_seq=?3)",
            params![id,revision,event_seq],
            |r| r.get::<_,bool>(0),
        )?;
        if current {
            return Ok(());
        }
        let tail_start = total.saturating_sub(TAIL_MESSAGES);
        let mut start = tail_start;
        let mut bytes = None;
        if self.can_display_tail_for_id(Some(id))? {
            if tail_start > 0 {
                start = viewport_page_start(&tx, id, tail_start)?;
            }
            let prepare = |from| self.prepare_page(&tx, id, from, Path::new(&project));
            if start < tail_start {
                bytes = match prepare(start) {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        log::warn!("Prepare expanded transcript viewport: {error}");
                        None
                    }
                };
            }
            if bytes.is_none() {
                start = tail_start;
                bytes = prepare(start)?;
            }
        }
        tx.commit()?;
        // No reader can publish a stale page after a concurrent append or a receipt.
        self.connection.execute(
            &format!("INSERT INTO transcript_viewports(session_id,revision,event_seq,from_beginning,items)
             SELECT session_id,revision,?3,?4,?5 FROM transcript_documents WHERE session_id=?1 AND revision=?2
             AND ({PRESENTATION_STAMP})=?3
             ON CONFLICT(session_id) DO UPDATE SET revision=excluded.revision,event_seq=excluded.event_seq,
               from_beginning=excluded.from_beginning,items=excluded.items"),
            params![id,revision,event_seq,complete && start==0,bytes],
        )?;
        Ok(())
    }

    fn prepare_page(
        &self,
        tx: &Transaction<'_>,
        id: i64,
        start: usize,
        project: &Path,
    ) -> Result<Option<Vec<u8>>, Error> {
        match self.viewport_messages(tx, id, start)? {
            Some(messages) => self.encode_page(&messages, project),
            None => Ok(None),
        }
    }

    fn viewport_messages(
        &self,
        tx: &Transaction<'_>,
        id: i64,
        start: usize,
    ) -> Result<Option<Vec<serde_json::Value>>, Error> {
        let bytes: i64 = tx.query_row(
            "SELECT coalesce(sum(octet_length(body)),0)
             FROM transcript_messages WHERE session_id=?1 AND ordinal>=?2",
            params![id, start],
            |r| r.get(0),
        )?;
        if bytes > MAX_INPUT_BYTES {
            return Ok(None);
        }
        let mut stmt = tx.prepare_cached(&format!(
            "{TRANSCRIPT_SELECT} WHERE m.session_id=?1 AND m.ordinal>=?2 ORDER BY m.ordinal"
        ))?;
        let mut rows = stmt.query(params![id, start])?;
        let mut messages = Vec::new();
        let mut remaining = (MAX_INPUT_BYTES - bytes) as usize;
        while let Some(row) = rows.next()? {
            // Legacy receipt images can contain inline base64. Check their stored
            // size before deserializing, then charge each selected image below.
            let mut link_budget = remaining;
            for column in [1, 3] {
                if let rusqlite::types::ValueRef::Text(json) = row.get_ref(column)?
                    && json != b"[]"
                {
                    let Some(left) = link_budget.checked_sub(json.len()) else {
                        return Ok(None);
                    };
                    link_budget = left;
                }
            }
            let decoded = self.decode_message_with_images(row, |image| {
                Ok(inline_viewport_image(image, &mut remaining)?)
            });
            match decoded {
                Ok(message) => messages.push(message),
                Err(error)
                    if error
                        .downcast_ref::<io::Error>()
                        .is_some_and(|e| e.kind() == io::ErrorKind::FileTooLarge) =>
                {
                    return Ok(None);
                }
                Err(error) => return Err(error),
            }
        }
        Ok(Some(messages))
    }

    pub(super) fn schedule_viewport_backfill(&self) {
        let pending = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM meta WHERE key='transcript_viewport_backfill')",
            [],
            |r| r.get::<_, bool>(0),
        );
        if matches!(pending, Ok(true)) {
            self.queue_viewport_preparation(None);
        }
    }

    fn queue_viewport_preparation(&self, id: Option<i64>) {
        let Some(path) = self
            .connection
            .path()
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
        else {
            return;
        };
        let mut queues = QUEUES.lock().unwrap_or_else(|e| e.into_inner());
        let running = queues.contains_key(&path);
        let work = queues.entry(path.clone()).or_default();
        if let Some(id) = id {
            work.sessions.insert(id);
        } else {
            work.backfill = true;
        }
        if running {
            return;
        }
        let key = path.clone();
        let images = self.image_directory.clone();
        if let Err(error) = thread::Builder::new()
            .name("transcript-viewport-preparation".into())
            .spawn(move || {
                let result = drain_preparation(&path, images);
                if let Err(err) = result {
                    QUEUES
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(&path);
                    log::warn!("Prepare transcript viewports: {err}");
                }
            })
        {
            queues.remove(&key);
            log::warn!("Start transcript viewport preparation: {error}");
        }
    }
}

/// The message body omits externalized image data. Charge its padded base64
/// length, as the reconstructed adapter message contains that whole string.
fn inline_viewport_image(image: PromptImage, remaining: &mut usize) -> io::Result<PromptImage> {
    use base64::Engine as _;

    if let Some(path) = &image.path {
        let bytes = read_viewport_image_file(path, remaining)?;
        return Ok(PromptImage::new(
            base64::engine::general_purpose::STANDARD.encode(bytes),
            image.mime_type,
        ));
    }
    *remaining = remaining
        .checked_sub(image.data.len())
        .ok_or(io::ErrorKind::FileTooLarge)?;
    Ok(image)
}

fn read_viewport_image_file(path: &Path, remaining: &mut usize) -> io::Result<Vec<u8>> {
    use io::Read as _;
    let file = std::fs::File::open(path)?;
    let max_raw = *remaining / 4 * 3;
    if file.metadata()?.len() > max_raw as u64 {
        return Err(io::ErrorKind::FileTooLarge.into());
    }
    let mut bytes = Vec::new();
    file.take(max_raw as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > max_raw {
        return Err(io::ErrorKind::FileTooLarge.into());
    }
    charge_viewport_image(bytes.len(), remaining)?;
    Ok(bytes)
}

fn charge_viewport_image(raw_bytes: usize, remaining: &mut usize) -> io::Result<()> {
    let encoded = base64::encoded_len(raw_bytes, true).ok_or(io::ErrorKind::FileTooLarge)?;
    *remaining = remaining
        .checked_sub(encoded)
        .ok_or(io::ErrorKind::FileTooLarge)?;
    Ok(())
}

fn drain_preparation(path: &Path, images: PathBuf) -> Result<(), Error> {
    let connection = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(DATABASE_BUSY_TIMEOUT)?;
    connection.pragma_update(None, "foreign_keys", true)?;
    let store = StateStore {
        connection,
        image_directory: images,
    };
    loop {
        let mut work = {
            let mut queues = QUEUES.lock().unwrap_or_else(|e| e.into_inner());
            let Some(entry) = queues.get_mut(path) else {
                return Ok(());
            };
            if !entry.backfill && entry.sessions.is_empty() {
                queues.remove(path);
                return Ok(());
            }
            std::mem::take(entry)
        };
        if work.backfill {
            let mut ids = store
                .connection
                .prepare("SELECT session_id FROM transcript_documents")?;
            work.sessions.extend(
                ids.query_map([], |r| r.get::<_, i64>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?,
            );
        }
        let mut succeeded = true;
        for id in work.sessions {
            if let Err(error) = store.prepare_viewport(id) {
                succeeded = false;
                log::warn!("Prepare session viewport: {error}");
            }
        }
        if work.backfill && succeeded {
            store.connection.execute(
                "DELETE FROM meta WHERE key='transcript_viewport_backfill'",
                [],
            )?;
        }
    }
}

fn coverage(complete: bool) -> agents::HistoryCoverage {
    if complete {
        agents::HistoryCoverage::Complete
    } else {
        agents::HistoryCoverage::Suffix
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ViewportImage {
    row: usize,
    hash: String,
    mime_type: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ViewportBody {
    #[serde(default, skip_serializing_if = "MarkdownIndex::is_empty")]
    markdown: MarkdownIndex,
    items: Vec<Arc<TranscriptItem>>,
    images: Vec<ViewportImage>,
}

impl StateStore {
    fn decode_viewport(&self, bytes: &[u8]) -> Result<(ConversationState, MarkdownIndex), Error> {
        if bytes.len() > MAX_VIEWPORT_BYTES {
            return Err(io::Error::from(io::ErrorKind::FileTooLarge).into());
        }
        let mut remaining = MAX_INPUT_BYTES as usize - bytes.len();
        let ViewportBody {
            mut items,
            images,
            mut markdown,
        } = serde_json::from_slice(bytes)?;
        let mut decoded: HashMap<_, Arc<farcaster_conversation::EncodedImage>> = HashMap::new();
        for link in images {
            let item = items
                .get_mut(link.row)
                .ok_or("invalid viewport image row")?;
            let key = (link.hash, link.mime_type);
            let image = if let Some(image) = decoded.get(&key) {
                charge_viewport_image(image.bytes().len(), &mut remaining)?;
                Arc::clone(image)
            } else {
                let stored = crate::images::StoredImage::File {
                    attachment: key.0.clone(),
                    mime_type: key.1.clone(),
                };
                let prompt = self.decode_prompt_image(stored)?;
                let path = prompt
                    .path
                    .as_deref()
                    .ok_or("missing viewport image path")?;
                let image = Arc::new(
                    farcaster_conversation::EncodedImage::new(
                        read_viewport_image_file(path, &mut remaining)?,
                        &key.1,
                    )
                    .ok_or("invalid viewport image")?,
                );
                decoded.insert(key, Arc::clone(&image));
                image
            };
            Arc::make_mut(&mut Arc::make_mut(item).images).push(image);
        }
        markdown.retain(|index, structure| {
            items
                .get(*index)
                .is_some_and(|item| structure.valid_for(&item.text))
        });
        let mut content = ConversationState::default();
        content.items = items.into_iter().collect();
        Ok((content, markdown))
    }

    fn viewport_image(
        &self,
        row: usize,
        image: &farcaster_conversation::EncodedImage,
    ) -> Result<ViewportImage, Error> {
        use sha2::{Digest, Sha256};
        let hash: String = Sha256::digest(image.bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        // A local user attachment normally already exists under this digest.
        // Only older inline records and adapter-only attachments need materializing.
        if !self.image_directory.join(&hash).is_file() {
            use base64::Engine;
            let data = base64::engine::general_purpose::STANDARD.encode(image.bytes());
            self.encode_prompt_image(&PromptImage::new(data, image.mime_type().to_owned()))?;
        }
        Ok(ViewportImage {
            row,
            hash,
            mime_type: image.mime_type().to_owned(),
        })
    }

    fn encode_page(
        &self,
        messages: &[serde_json::Value],
        project: &Path,
    ) -> Result<Option<Vec<u8>>, Error> {
        let mut source = ConversationState::default();
        source.replace_history(messages);
        if source.items.is_empty() || source.items.len() > MAX_VIEWPORT_ITEMS {
            return Ok(None);
        }
        for item in source.items.iter() {
            if (!item.images.is_empty() && item.kind == TranscriptKind::Tool)
                || farcaster_reviews::artifact::from_item(item).is_some()
                || farcaster_reviews::artifact::hydration_result(item, project).is_some()
            {
                return Ok(None);
            }
        }
        // Keep singleton activity rows exact: a single command can wrap and show its full
        // command label. Groups use a collapsed category header with a separate change tree.
        let groupable = |item: &TranscriptItem| {
            matches!(item.kind, TranscriptKind::Tool | TranscriptKind::Thinking)
        };
        let mut images = Vec::new();
        let mut items = Vec::with_capacity(source.items.len());
        for (index, item) in source.items.iter().enumerate() {
            let grouped = groupable(item)
                && (index
                    .checked_sub(1)
                    .and_then(|i| source.items.get(i))
                    .is_some_and(|i| groupable(i))
                    || source.items.get(index + 1).is_some_and(|i| groupable(i)));
            for image in item.images.iter() {
                images.push(self.viewport_image(index, image)?);
            }
            items.push(Arc::new(if grouped {
                item.for_collapsed_viewport()
            } else {
                item.as_ref().clone()
            }));
        }
        if items.iter().map(|item| item.text.len()).sum::<usize>() > MAX_VIEWPORT_BYTES {
            return Ok(None);
        }
        let markdown = items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                matches!(item.kind, TranscriptKind::Assistant | TranscriptKind::User)
                    && (item.text.len() > 2048 || item.text.lines().nth(64).is_some())
            })
            .filter_map(|(i, item)| {
                MarkdownStructure::parse(&item.text).map(|structure| (i, structure))
            })
            .collect();
        let bytes = serde_json::to_vec(&ViewportBody {
            items,
            images,
            markdown,
        })?;
        Ok((bytes.len() <= MAX_VIEWPORT_BYTES).then_some(bytes))
    }
}

#[cfg(test)]
#[path = "transcript_viewport_tests.rs"]
mod tests;

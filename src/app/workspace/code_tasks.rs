use std::{collections::HashMap, path::PathBuf, time::Duration};

use gpui::{Context, Task, Window};

use crate::app::{
    FarcasterApp,
    composer::{sessions::draft_target, submissions::PendingSubmission},
    project_registry,
};
use crate::runtime::{RuntimeCommand, TaskSettings};

#[derive(Clone)]
struct TaskChat {
    target: String,
    project: PathBuf,
    session: Option<PathBuf>,
    new_task: bool,
}

struct TaskNotice {
    chat: TaskChat,
    result: Option<bool>,
    _expiry: Option<Task<()>>,
}

#[derive(Default)]
pub(in crate::app) struct CodeTasks {
    pending: HashMap<String, TaskChat>,
    notice: Option<TaskNotice>,
}

impl CodeTasks {
    pub(in crate::app) fn notice_message(&self) -> Option<&'static str> {
        let notice = self.notice.as_ref()?;
        Some(match (notice.chat.new_task, notice.result) {
            (true, None) => "Starting task…",
            (true, Some(true)) => "Task started",
            (true, Some(false)) => "Task couldn’t start",
            (false, None) => "Sending comment…",
            (false, Some(true)) => "Comment sent",
            (false, Some(false)) => "Comment couldn’t send · Saved in chat draft",
        })
    }

    pub(in crate::app) fn associate(&mut self, target: &str, session: Option<&std::path::Path>) {
        let Some(session) = session else { return };
        if let Some(chat) = self.pending.get_mut(target) {
            chat.session = Some(session.to_path_buf());
        }
        if let Some(notice) = &mut self.notice
            && notice.chat.target == target
        {
            notice.chat.session = Some(session.to_path_buf());
        }
    }
}

impl FarcasterApp {
    pub(in crate::app) fn submit_code_comment(
        &mut self,
        destination: super::code_comment::CodeDestination,
        project: PathBuf,
        message: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = destination.target;
        if self.pending_submissions.contains_key(&target) {
            self.code_comment_error(
                "A message is still being sent to this chat. Try again shortly.".into(),
                cx,
            );
            return;
        }
        if !self.ensure_backend_trust(&destination.harness, &project, window, cx) {
            return;
        }
        let chat = TaskChat {
            target: target.clone(),
            project: project.clone(),
            session: destination
                .session
                .as_ref()
                .map(|session| session.path.clone()),
            new_task: false,
        };
        if let Err(error) = self.runtime.send(RuntimeCommand::SendToSession {
            target: target.clone(),
            session: destination.session,
            project,
            message: message.clone(),
        }) {
            self.code_comment_error(error, cx);
            return;
        }
        self.capture_composer_session(cx);
        if let Some(path) = crate::app::composer::submissions::inactive_session_for_target(
            &target,
            chat.session.as_deref(),
            &self.all_sessions,
        ) {
            self.set_session_active(path, cx);
        }
        self.track_code_submission(chat, message, cx);
        self.close_code_comment(window, cx);
        self.notify_session_rail(cx);
    }

    pub(in crate::app) fn submit_code_task(
        &mut self,
        settings: TaskSettings,
        message: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.ensure_backend_trust(&settings.harness, &settings.project, window, cx) {
            return;
        }
        let draft = project_registry::new_draft(settings.project.clone(), &settings.harness)
            .and_then(|draft| {
                let mut registry = crate::projects::Registry {
                    projects: self.projects.clone(),
                    excluded_projects: self.excluded_projects.clone(),
                    drafts: self.drafts.clone(),
                };
                registry.drafts.insert(0, draft.clone());
                project_registry::save(&registry)?;
                self.drafts = registry.drafts;
                Ok(draft)
            });
        let draft = match draft {
            Ok(draft) => draft,
            Err(error) => {
                self.code_comment_error(error, cx);
                return;
            }
        };
        let target = draft_target(&draft.id);
        self.draft_session_ids
            .insert(draft.id.clone(), draft.app_session_id);
        let chat = TaskChat {
            target: target.clone(),
            project: settings.project.clone(),
            session: None,
            new_task: true,
        };
        match self.runtime.send(RuntimeCommand::StartTask {
            id: draft.id,
            settings,
            message: message.clone(),
        }) {
            Ok(()) => self.track_code_submission(chat, message, cx),
            Err(_) => {
                self.composer_sessions
                    .restore_submitted_text(&target, message);
                self.show_code_task_notice(chat, Some(false), cx);
            }
        }
        self.close_code_comment(window, cx);
        self.notify_session_rail(cx);
    }

    fn track_code_submission(&mut self, chat: TaskChat, message: String, cx: &mut Context<Self>) {
        let target = &chat.target;
        self.begin_draft_submission(target, &message);
        self.composer_sessions.record_submission(target, &message);
        self.pending_submissions.insert(
            target.clone(),
            PendingSubmission {
                submitted_target: target.clone(),
                text: message,
                images: Vec::new(),
                pastes: Vec::new(),
                append_on_failure: !chat.new_task,
                result: None,
            },
        );
        self.code_tasks.pending.insert(target.clone(), chat.clone());
        self.show_code_task_notice(chat, None, cx);
    }

    pub(in crate::app) fn code_task_result(
        &mut self,
        target: &str,
        accepted: bool,
        session: Option<&std::path::Path>,
        cx: &mut Context<Self>,
    ) {
        self.code_tasks.associate(target, session);
        let Some(chat) = self.code_tasks.pending.remove(target) else {
            return;
        };
        self.show_code_task_notice(chat, Some(accepted), cx);
    }

    fn show_code_task_notice(
        &mut self,
        chat: TaskChat,
        result: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        let expiry = (result == Some(true)).then(|| {
            cx.spawn(async move |weak, cx| {
                cx.background_executor().timer(Duration::from_secs(6)).await;
                let _ = weak.update(cx, |this, cx| this.dismiss_code_task_notice(cx));
            })
        });
        self.code_tasks.notice = Some(TaskNotice {
            chat,
            result,
            _expiry: expiry,
        });
        cx.notify();
    }

    pub(in crate::app) fn dismiss_code_task_notice(&mut self, cx: &mut Context<Self>) {
        self.code_tasks.notice = None;
        cx.notify();
    }

    pub(in crate::app) fn open_code_task_chat(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(notice) = self.code_tasks.notice.take() else {
            return;
        };
        let chat = notice.chat;
        if let Some(path) = chat.session {
            self.select_session_and_focus(path, chat.project, window, cx);
        } else if let Some(id) = chat.target.strip_prefix("draft:") {
            self.resume_draft_and_focus(id.to_owned(), chat.project, window, cx);
        }
        self.show_chat_surface(window, cx);
    }
}

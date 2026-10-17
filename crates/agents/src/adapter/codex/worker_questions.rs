use super::*;

pub(super) const QUESTION_SUBMISSION_PREFIX: &str = "question-answer:";

impl CodexWorkerSession {
    pub(super) fn answer_async_question(
        &mut self,
        response: crate::QuestionResponse,
    ) -> Result<(), String> {
        if self
            .async_answers
            .iter()
            .chain(self.question_submissions.values())
            .any(|answer| answer.id == response.id)
        {
            return Ok(());
        }
        if response.cancelled {
            self.pending_questions.remove(&response.id);
            self.events
                .push_back(WorkerEvent::InputResolved { id: response.id });
        } else {
            self.pending_questions[&response.id].async_reply(&response)?;
            self.async_answers.push_back(response);
        }
        Ok(())
    }

    pub(super) fn send_next_async_answer(&mut self) -> Result<(), String> {
        if self.async_answers.is_empty()
            || self.abort_cleanup.is_some()
            || self.abort_starting_turn
            || self.handoff.is_some()
        {
            return Ok(());
        }
        let Some(mode) = WorkerSendMode::for_peer(self.activity()) else {
            return Ok(());
        };
        if !self.caller_identity.try_activate() {
            return Ok(());
        }
        let response = self.async_answers.pop_front().unwrap();
        let text = self.pending_questions[&response.id].async_reply(&response)?;
        let submission_id = format!("{QUESTION_SUBMISSION_PREFIX}{}", uuid::Uuid::new_v4());
        let delivery = NativeInputDelivery {
            submission_id: Some(submission_id.clone()),
            mode,
            message: super::super::questions::async_reply_display(&text)
                .unwrap_or_else(|| text.clone()),
            images: vec![],
        };
        // Use the same admission, delivery, handoff and Abort tracking as composer input.
        self.send_input(
            vec![CodexUserInput::text(text)],
            mode,
            Some(&submission_id),
            delivery,
        )?;
        self.prompt_requests
            .insert(CodexRequestId::Number(self.next_id), submission_id.clone());
        self.question_submissions.insert(submission_id, response);
        Ok(())
    }

    pub(super) fn resolve_question_submission(&mut self, submission_id: &str) {
        if let Some(response) = self.question_submissions.remove(submission_id) {
            self.pending_questions.remove(&response.id);
            self.events
                .push_back(WorkerEvent::InputResolved { id: response.id });
        }
    }

    pub(super) fn cancel_async_answers(&mut self) {
        for response in self.async_answers.drain(..).chain(
            self.question_submissions
                .drain()
                .map(|(_, response)| response),
        ) {
            self.pending_questions.remove(&response.id);
            self.events
                .push_back(WorkerEvent::InputResolved { id: response.id });
        }
    }

    pub(super) fn question_ack(&mut self, id: &str, result: &Result<(), crate::PromptRejection>) {
        let Err(error) = result else {
            return;
        };
        let Some(response) = self.question_submissions.remove(id) else {
            return;
        };
        self.events.push_back(WorkerEvent::RequestFailed {
            operation: "Answer Codex question".into(),
            error: error.to_string(),
        });
        if let Some(pending) = self.pending_questions.get(&response.id) {
            let mut request = pending.request.clone();
            for question in &mut request.questions {
                if let Some(previous) = response.answers.get(&question.id) {
                    let label = previous.join("\n");
                    question.options.retain(|option| option.label != label);
                    question.options.insert(
                        0,
                        crate::QuestionOption {
                            label,
                            description: "Your previous answer. Select to retry.".into(),
                        },
                    );
                }
            }
            self.events
                .push_back(WorkerEvent::QuestionsRequested(request));
        }
    }
}

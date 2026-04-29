use crate::jobs_types::{AcceptedJob, JobReviewInput, JobReviewView};
use anima_runtime::support::now_ms;
use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct JobStore {
    accepted: HashMap<String, AcceptedJob>,
    reviews: HashMap<String, JobReviewView>,
}

impl JobStore {
    pub fn register_accepted_job(&mut self, job: AcceptedJob) {
        self.accepted.insert(job.job_id.clone(), job);
    }

    pub fn accepted_job(&self, job_id: &str) -> Option<AcceptedJob> {
        self.accepted.get(job_id).cloned()
    }

    pub fn accepted_jobs(&self) -> impl Iterator<Item = &AcceptedJob> {
        self.accepted.values()
    }

    pub fn parent_job_id_by_trace(&self, trace_id: &str) -> Option<String> {
        self.accepted
            .values()
            .find(|job| job.trace_id == trace_id && job.parent_job_id.is_some())
            .and_then(|job| job.parent_job_id.clone())
    }

    pub fn record_review(&mut self, job_id: String, input: JobReviewInput) -> JobReviewView {
        let review = JobReviewView {
            verdict: input.user_verdict,
            reason: input.reason.filter(|value| !value.trim().is_empty()),
            note: input.note.filter(|value| !value.trim().is_empty()),
            reviewed_at_ms: now_ms(),
        };
        self.reviews.insert(job_id, review.clone());
        review
    }

    pub fn review_for(&self, job_id: &str) -> Option<JobReviewView> {
        self.reviews.get(job_id).cloned()
    }

    pub fn remove_by_chat_id(&mut self, chat_id: &str) {
        let job_ids: Vec<String> = self
            .accepted
            .iter()
            .filter(|(_, j)| j.chat_id.as_deref() == Some(chat_id))
            .map(|(id, _)| id.clone())
            .collect();
        for id in &job_ids {
            self.accepted.remove(id);
            self.reviews.remove(id);
        }
    }
}

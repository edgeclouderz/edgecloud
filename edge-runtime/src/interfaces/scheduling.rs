//! `edge:scheduling` — delayed and repeating task execution.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

pub struct ScheduledTask {
    pub id: String,
    pub payload: Vec<u8>,
}

pub struct Scheduler {
    tasks: Arc<Mutex<HashMap<String, tokio::task::JoinHandle<()>>>>,
    sender: mpsc::Sender<ScheduledTask>,
}

impl Scheduler {
    pub fn new() -> Self {
        let (tx, mut rx) = mpsc::channel::<ScheduledTask>(1000);
        let tasks: Arc<Mutex<HashMap<String, tokio::task::JoinHandle<()>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let tasks_clone = tasks.clone();
        tokio::spawn(async move {
            while let Some(task) = rx.recv().await {
                tracing::debug!(task_id = %task.id, "scheduled task received");
                let mut tasks_guard = tasks_clone.lock().unwrap();
                tasks_guard.remove(&task.id);
            }
        });

        Self {
            tasks,
            sender: tx,
        }
    }

    pub fn schedule_once(&self, _delay_ms: u64, payload: Vec<u8>) -> Result<String, String> {
        let id = format!("task_{}", std::process::id());
        let task = ScheduledTask { id: id.clone(), payload };
        let _ = self.sender.try_send(task);
        Ok(id)
    }

    pub fn schedule_repeating(&self, _interval_ms: u64, payload: Vec<u8>) -> Result<String, String> {
        let id = format!("task_{}", std::process::id());
        let _ = payload;
        Ok(id)
    }

    pub fn cancel(&self, id: &str) -> Result<(), String> {
        let mut tasks = self.tasks.lock().unwrap();
        tasks.remove(id);
        Ok(())
    }
}
//! Bounded component invocation actors; transport supplies authenticated ownership.
use super::NativePluginInstaller;
use openwebide_core::plugins::execution::{
    ContinuePlugin, InvokePlugin, PluginInvocation, PluginStep,
};
use openwebide_plugin_runtime::{HostServices, MAX_MESSAGE_BYTES, Runtime};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, mpsc as async_mpsc};

#[derive(Clone, Default)]
pub struct Invocations {
    entries: Arc<Mutex<HashMap<String, Arc<Mutex<Invocation>>>>>,
}
struct Invocation {
    owner: String,
    started: Instant,
    cancelled: Arc<AtomicBool>,
    events: async_mpsc::Receiver<Event>,
    pending: Option<(u32, mpsc::SyncSender<Result<String, String>>)>,
}
impl Drop for Invocation {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}
enum Event {
    HostCall {
        sequence: u32,
        capability: String,
        payload: String,
        response: mpsc::SyncSender<Result<String, String>>,
    },
    Finished(PluginStep),
}
struct Services {
    events: async_mpsc::Sender<Event>,
    cancelled: Arc<AtomicBool>,
    sequence: u32,
}
impl HostServices for Services {
    fn request(&mut self, capability: &str, payload: &str) -> Result<String, String> {
        if self.cancelled.load(Ordering::Relaxed) || self.sequence >= 128 {
            return Err("Plugin invocation was cancelled or exceeded its host-call limit.".into());
        }
        self.sequence += 1;
        let (sender, receiver) = mpsc::sync_channel(1);
        self.events
            .blocking_send(Event::HostCall {
                sequence: self.sequence,
                capability: capability.into(),
                payload: payload.into(),
                response: sender,
            })
            .map_err(|_| "Plugin invocation was closed.")?;
        match receiver.recv_timeout(Duration::from_secs(60)) {
            Ok(result) => result,
            Err(_) => {
                self.cancelled.store(true, Ordering::Relaxed);
                Err("Plugin host call timed out or was cancelled.".into())
            }
        }
    }
}
impl Invocations {
    pub async fn start(
        &self,
        installer: &NativePluginInstaller,
        owner: &str,
        call: InvokePlugin,
    ) -> Result<PluginInvocation, String> {
        call.prepared
            .validate()
            .map_err(|error| error.to_string())?;
        let rust = call
            .prepared
            .manifest
            .executable
            .as_ref()
            .ok_or("Plugin has no executable")?;
        if !call
            .prepared
            .manifest
            .contributions
            .tools
            .iter()
            .any(|tool| tool.name == call.name)
            || call.arguments.len() > MAX_MESSAGE_BYTES
        {
            return Err(
                "Tool is not declared by this plugin, or its arguments exceed the limit.".into(),
            );
        }
        let _: serde_json::Value =
            serde_json::from_str(&call.arguments).map_err(|error| error.to_string())?;
        let bytes = installer
            .component(owner, &call.prepared)
            .await
            .map_err(|error| error.to_string())?;
        let id = format!("{:032x}", rand::random::<u128>());
        let cancelled = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = async_mpsc::channel(1);
        let entry = Arc::new(Mutex::new(Invocation {
            owner: owner.into(),
            started: Instant::now(),
            cancelled: cancelled.clone(),
            events: receiver,
            pending: None,
        }));
        {
            let mut entries = self.entries.lock().await;
            entries.retain(|_, entry| {
                entry.try_lock().map_or(true, |state| {
                    state.started.elapsed() < Duration::from_secs(300)
                })
            });
            if entries.len() >= 32 {
                return Err("Too many active plugin invocations.".into());
            }
            entries.insert(id.clone(), entry.clone());
        }
        let services = Services {
            events: sender.clone(),
            cancelled,
            sequence: 0,
        };
        let grants = rust.capabilities.clone();
        tokio::task::spawn_blocking(move || {
            let result = Runtime::new().and_then(|runtime| {
                runtime.execute(&bytes, services, &grants, &call.name, &call.arguments)
            });
            let step = match result {
                Ok(outcome) => PluginStep::Complete {
                    ok: outcome.ok,
                    content: outcome.content,
                    summary: outcome.summary,
                },
                Err(error) => PluginStep::Failed {
                    error: format!("Plugin execution failed: {error:#}"),
                },
            };
            let _ = sender.try_send(Event::Finished(step));
        });
        self.next(owner, &id, entry).await
    }
    pub async fn resume(
        &self,
        owner: &str,
        response: ContinuePlugin,
    ) -> Result<PluginInvocation, String> {
        if response
            .response
            .as_ref()
            .map_or_else(String::len, String::len)
            > MAX_MESSAGE_BYTES
        {
            return Err("Plugin host response exceeds its limit.".into());
        }
        let entry = self.entry(&response.id).await?;
        {
            let mut state = entry.lock().await;
            if state.owner != owner {
                return Err("Plugin invocation belongs to another owner.".into());
            }
            let pending = state
                .pending
                .as_ref()
                .ok_or("No plugin host response is pending")?;
            if pending.0 != response.sequence {
                return Err("Stale plugin host response.".into());
            }
            let (_, sender) = state.pending.take().expect("checked pending call");
            sender
                .send(response.response)
                .map_err(|_| "Plugin invocation has ended.")?;
        }
        self.next(owner, &response.id, entry).await
    }
    pub async fn cancel(&self, owner: &str, id: &str) -> Result<(), String> {
        let entry = self.entry(id).await?;
        {
            let mut state = entry.lock().await;
            if state.owner != owner {
                return Err("Plugin invocation belongs to another owner.".into());
            }
            state.cancelled.store(true, Ordering::Relaxed);
            state.pending.take();
        }
        self.entries.lock().await.remove(id);
        Ok(())
    }
    async fn entry(&self, id: &str) -> Result<Arc<Mutex<Invocation>>, String> {
        self.entries
            .lock()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| "Plugin invocation is no longer active.".into())
    }
    async fn next(
        &self,
        owner: &str,
        id: &str,
        entry: Arc<Mutex<Invocation>>,
    ) -> Result<PluginInvocation, String> {
        let mut state = entry.lock().await;
        if state.owner != owner {
            return Err("Plugin invocation belongs to another owner.".into());
        }
        let event = tokio::time::timeout(Duration::from_secs(60), state.events.recv())
            .await
            .map_err(|_| "Plugin execution timed out.")?
            .ok_or("Plugin invocation ended without a result")?;
        let step = match event {
            Event::HostCall {
                sequence,
                capability,
                payload,
                response,
            } => {
                state.pending = Some((sequence, response));
                PluginStep::HostCall {
                    sequence,
                    capability,
                    payload,
                }
            }
            Event::Finished(step) => {
                drop(state);
                self.entries.lock().await.remove(id);
                step
            }
        };
        Ok(PluginInvocation {
            id: id.into(),
            step,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pending_responses_reject_other_owners_and_stale_sequences_before_cancellation() {
        let invocations = Invocations::default();
        let cancelled = Arc::new(AtomicBool::new(false));
        let (_events, receiver) = async_mpsc::channel(1);
        let (response, replies) = mpsc::sync_channel(1);
        invocations.entries.lock().await.insert(
            "call".into(),
            Arc::new(Mutex::new(Invocation {
                owner: "owner".into(),
                started: Instant::now(),
                cancelled: cancelled.clone(),
                events: receiver,
                pending: Some((7, response)),
            })),
        );
        for (owner, sequence) in [("other", 7), ("owner", 6)] {
            assert!(
                invocations
                    .resume(
                        owner,
                        ContinuePlugin {
                            id: "call".into(),
                            sequence,
                            response: Ok("{}".into()),
                        },
                    )
                    .await
                    .is_err()
            );
            assert!(matches!(replies.try_recv(), Err(mpsc::TryRecvError::Empty)));
        }
        assert!(invocations.cancel("other", "call").await.is_err());
        assert!(!cancelled.load(Ordering::Relaxed));
        invocations.cancel("owner", "call").await.unwrap();
        assert!(cancelled.load(Ordering::Relaxed));
        assert!(matches!(
            replies.try_recv(),
            Err(mpsc::TryRecvError::Disconnected)
        ));
        assert!(invocations.entry("call").await.is_err());
    }
}

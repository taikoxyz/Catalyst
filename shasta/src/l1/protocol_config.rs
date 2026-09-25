use anyhow::Error;
use common::utils::cancellation_token::CancellationToken;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;
use taiko_bindings::inbox::IInbox::Config;
use taiko_protocol::shasta::constants::{
    max_anchor_offset_for_chain, timestamp_max_offset_for_chain,
};
use tokio::task::JoinHandle;
use tracing::{info, warn};

/// Protocol parameters the node needs while building and proposing blocks.
///
/// `basefee_sharing_pctg` is an immutable of the inbox *implementation*, so it changes whenever the
/// DAO upgrades the inbox proxy. It lives behind an `Arc` so that every clone of this config (the
/// block advancer keeps one) sees the value written by [`Self::spawn_basefee_sharing_pctg_refresh`].
#[derive(Clone, Default)]
pub struct ProtocolConfig {
    basefee_sharing_pctg: Arc<AtomicU8>,
    max_anchor_offset: u64,
    timestamp_max_offset: u64,
}

impl ProtocolConfig {
    pub fn from(chain_id: u64, inbox_config: &Config) -> Self {
        Self::new(chain_id, inbox_config.basefeeSharingPctg)
    }

    pub fn new(chain_id: u64, basefee_sharing_pctg: u8) -> Self {
        Self {
            basefee_sharing_pctg: Arc::new(AtomicU8::new(basefee_sharing_pctg)),
            max_anchor_offset: max_anchor_offset_for_chain(chain_id),
            timestamp_max_offset: timestamp_max_offset_for_chain(chain_id),
        }
    }

    pub fn get_basefee_sharing_pctg(&self) -> u8 {
        self.basefee_sharing_pctg.load(Ordering::Relaxed)
    }

    /// Stores a freshly read percentage, visible to every clone, and returns the previous one.
    pub fn set_basefee_sharing_pctg(&self, basefee_sharing_pctg: u8) -> u8 {
        self.basefee_sharing_pctg
            .swap(basefee_sharing_pctg, Ordering::Relaxed)
    }

    pub fn get_max_anchor_height_offset(&self) -> u64 {
        self.max_anchor_offset
    }

    pub fn get_timestamp_max_offset(&self) -> u64 {
        self.timestamp_max_offset
    }

    /// Keeps `basefee_sharing_pctg` in sync with the inbox without a restart.
    ///
    /// Every `period`, `fetch` reads the percentage the inbox currently reports and the value every
    /// clone of this config sees is updated. A failed read keeps the last value and is retried at the
    /// next tick. The task ends when `cancel_token` fires.
    ///
    /// The percentage is stamped into every block's `extraData`, and the driver derives the blocks
    /// of a proposal with the value the inbox emits when the proposal lands, so a node still on a
    /// stale value after an inbox upgrade produces blocks that are replaced once proposed.
    pub fn spawn_basefee_sharing_pctg_refresh<F, Fut>(
        &self,
        fetch: F,
        period: Duration,
        cancel_token: CancellationToken,
    ) -> JoinHandle<()>
    where
        F: Fn() -> Fut + Send + 'static,
        Fut: Future<Output = Result<u8, Error>> + Send,
    {
        let config = self.clone();
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval_at(tokio::time::Instant::now() + period, period);
            loop {
                tokio::select! {
                    _ = interval.tick() => {}
                    _ = cancel_token.cancelled() => {
                        info!("Shutdown signal received, exiting basefee sharing refresh loop...");
                        return;
                    }
                }

                match fetch().await {
                    Ok(basefee_sharing_pctg) => {
                        let previous = config.set_basefee_sharing_pctg(basefee_sharing_pctg);
                        if previous != basefee_sharing_pctg {
                            info!(
                                "Inbox basefee sharing percentage changed from {} to {}; blocks built from now on carry the new value",
                                previous, basefee_sharing_pctg
                            );
                        }
                    }
                    Err(err) => {
                        warn!(
                            "Failed to refresh the basefee sharing percentage from the inbox, keeping {}: {}",
                            config.get_basefee_sharing_pctg(),
                            err
                        );
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::metrics::Metrics;
    use std::sync::Mutex;

    const CHAIN_ID: u64 = 167_000;

    #[test]
    fn clones_share_the_basefee_sharing_pctg() {
        let config = ProtocolConfig::new(CHAIN_ID, 75);
        let block_advancer_copy = config.clone();

        assert_eq!(config.set_basefee_sharing_pctg(100), 75);

        assert_eq!(config.get_basefee_sharing_pctg(), 100);
        assert_eq!(block_advancer_copy.get_basefee_sharing_pctg(), 100);
    }

    #[test]
    fn from_takes_the_percentage_from_the_inbox_config() {
        let inbox_config = Config {
            basefeeSharingPctg: 75,
            ..Default::default()
        };
        let config = ProtocolConfig::from(CHAIN_ID, &inbox_config);

        assert_eq!(config.get_basefee_sharing_pctg(), 75);
        assert_eq!(
            config.get_max_anchor_height_offset(),
            max_anchor_offset_for_chain(CHAIN_ID)
        );
        assert_eq!(
            config.get_timestamp_max_offset(),
            timestamp_max_offset_for_chain(CHAIN_ID)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn refresh_tracks_the_inbox_and_keeps_the_last_value_on_errors() {
        let period = Duration::from_secs(12);
        let inbox: Arc<Mutex<Result<u8, String>>> = Arc::new(Mutex::new(Ok(75)));
        let config = ProtocolConfig::new(CHAIN_ID, 75);
        let cancel_token = CancellationToken::new(Arc::new(Metrics::new()));

        let handle = config.spawn_basefee_sharing_pctg_refresh(
            {
                let inbox = inbox.clone();
                move || {
                    let inbox = inbox.clone();
                    async move {
                        inbox
                            .lock()
                            .expect("inbox lock")
                            .clone()
                            .map_err(Error::msg)
                    }
                }
            },
            period,
            cancel_token.clone(),
        );

        // The inbox is upgraded: the next tick picks the new percentage up.
        *inbox.lock().expect("inbox lock") = Ok(100);
        assert_eq!(config.get_basefee_sharing_pctg(), 75);
        tokio::time::sleep(period + Duration::from_millis(1)).await;
        assert_eq!(config.get_basefee_sharing_pctg(), 100);

        // A failed read keeps the last value.
        *inbox.lock().expect("inbox lock") = Err("rpc down".to_string());
        tokio::time::sleep(period).await;
        assert_eq!(config.get_basefee_sharing_pctg(), 100);

        // The loop exits on shutdown.
        cancel_token.cancel();
        tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .expect("refresh loop did not exit on cancellation")
            .expect("refresh loop panicked");
    }
}

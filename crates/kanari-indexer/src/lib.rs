pub mod db;

pub use db::{IndexerReader, IndexerStore};
use kanari_types::effects::TransactionEffects;
use log::{error, info};
use tokio::sync::mpsc;

/// ตัวจัดการ Indexer Service
pub struct KanariIndexer {
    receiver: mpsc::Receiver<Vec<TransactionEffects>>,
    db_path: String,
}

impl KanariIndexer {
    /// สร้าง Indexer คู่กับ Transmitter เอาไว้ส่งข้อมูลให้มัน
    pub fn new(db_path: &str, buffer_size: usize) -> (Self, mpsc::Sender<Vec<TransactionEffects>>) {
        let (sender, receiver) = mpsc::channel(buffer_size);
        let indexer = Self {
            receiver,
            db_path: db_path.to_string(),
        };
        (indexer, sender)
    }

    /// สั่งรัน Indexer ใน Background Thread
    pub async fn run(mut self) {
        // ให้ SQLite วิ่งใน thread แยกต่างหาก เพื่อไม่ให้บล็อก Async Runtime ของ Tokio
        let db_path = self.db_path.clone();

        let (db_sender, mut db_receiver) = mpsc::channel::<Vec<TransactionEffects>>(1000);

        // Spawn Blocking thread สำหรับเขียน SQLite
        tokio::task::spawn_blocking(move || {
            let mut store = match IndexerStore::new(&db_path) {
                Ok(s) => s,
                Err(e) => {
                    error!("[INDEXER] Failed to open SQLite DB: {}", e);
                    return;
                }
            };
            info!("[INDEXER] SQLite connected at {}", db_path);

            while let Some(effects_batch) = db_receiver.blocking_recv() {
                let batch_size = effects_batch.len();
                if let Err(e) = store.save_effects(effects_batch) {
                    error!("[INDEXER] Failed to save batch to SQLite: {}", e);
                } else {
                    log::debug!("[INDEXER] Saved {} effects to SQLite", batch_size);
                }
            }
        });

        // Loop หลักสำหรับรับ Effects จาก Core Engine แล้วส่งต่อให้ DB Thread
        while let Some(effects) = self.receiver.recv().await {
            if db_sender.send(effects).await.is_err() {
                error!("[INDEXER] DB thread channel closed unexpectedly");
                break;
            }
        }
    }
}

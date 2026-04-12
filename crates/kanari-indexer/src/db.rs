use anyhow::Result;
use kanari_types::effects::{ExecutionStatus, TransactionEffects};
use rusqlite::{Connection, OpenFlags, params};
use std::sync::Mutex;

pub struct IndexerStore {
    conn: Connection,
}

impl IndexerStore {
    /// สร้างการเชื่อมต่อและสร้างตารางหากยังไม่มี
    pub fn new(db_path: &str) -> Result<Self> {
        let conn = Connection::open(db_path)?;

        // สร้างตาราง Transactions
        conn.execute(
            "CREATE TABLE IF NOT EXISTS transactions (
                digest TEXT PRIMARY KEY,
                status TEXT NOT NULL,
                computation_cost INTEGER NOT NULL,
                storage_cost INTEGER NOT NULL,
                storage_rebate INTEGER NOT NULL
            )",
            [],
        )?;

        // สร้างตาราง Objects ที่ถูกอัปเดต
        conn.execute(
            "CREATE TABLE IF NOT EXISTS objects (
                object_id TEXT PRIMARY KEY,
                version INTEGER NOT NULL,
                digest TEXT NOT NULL,
                last_updated_by_tx TEXT NOT NULL
            )",
            [],
        )?;

        // สร้าง Index เพื่อให้ค้นหา Object ได้เร็วปรู๊ดปร๊าด
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_objects_version ON objects(version)",
            [],
        )?;

        Ok(Self { conn })
    }

    /// บันทึกใบเสร็จ TransactionEffects ลง SQLite
    pub fn save_effects(&mut self, effects_list: Vec<TransactionEffects>) -> Result<()> {
        // ใช้ Transaction เพื่อความเร็วในการ Insert ทีละเยอะๆ (Batch Insert)
        let tx = self.conn.transaction()?;

        {
            let mut stmt_tx = tx.prepare(
                "INSERT OR REPLACE INTO transactions 
                 (digest, status, computation_cost, storage_cost, storage_rebate) 
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;

            let mut stmt_obj = tx.prepare(
                "INSERT OR REPLACE INTO objects 
                 (object_id, version, digest, last_updated_by_tx) 
                 VALUES (?1, ?2, ?3, ?4)",
            )?;

            for effects in effects_list {
                let tx_digest_hex = hex::encode(effects.transaction_digest.0.0);

                let status_str = match effects.status {
                    ExecutionStatus::Success => "Success",
                    ExecutionStatus::Failure { .. } => "Failure",
                };

                // 1. บันทึกข้อมูล Transaction (แปลง u64 เป็น i64 สำหรับ SQLite)
                stmt_tx.execute(params![
                    tx_digest_hex,
                    status_str,
                    effects.gas_used.computation_cost as i64,
                    effects.gas_used.storage_cost as i64,
                    effects.gas_used.storage_rebate as i64,
                ])?;

                // 2. บันทึก Objects ที่ถูกสร้างหรือแก้ไข (Upsert)
                let all_changed_objects = effects.created.iter().chain(effects.mutated.iter());
                for obj_ref in all_changed_objects {
                    stmt_obj.execute(params![
                        obj_ref.object_id.to_hex_literal(),
                        obj_ref.version as i64,
                        hex::encode(obj_ref.digest.0.0),
                        tx_digest_hex,
                    ])?;
                }
            }
        }

        tx.commit()?;
        Ok(())
    }
}

// ==========================================
// ส่วนของการอ่านข้อมูล (Read-Only) สำหรับ RPC
// ==========================================

pub struct IndexerReader {
    conn: Mutex<Connection>,
}

impl IndexerReader {
    /// เปิดการเชื่อมต่อแบบ Read-Only เพื่อไม่ให้ไปล็อก (Lock) ฐานข้อมูลตอนที่โหนดหลักกำลังเขียน
    pub fn new(db_path: &str) -> Result<Self> {
        let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// ค้นหาสถานะของธุรกรรมจาก SQLite
    pub fn get_transaction_status(&self, digest_hex: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT status FROM transactions WHERE digest = ?1")?;
        let mut rows = stmt.query([digest_hex])?;

        if let Some(row) = rows.next()? {
            let status: String = row.get(0)?;
            Ok(Some(status))
        } else {
            Ok(None)
        }
    }

    /// ค้นหา Object ล่าสุดว่าถูกอัปเดตจากธุรกรรมไหน
    pub fn get_object_last_tx(&self, object_id_hex: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT last_updated_by_tx FROM objects WHERE object_id = ?1")?;
        let mut rows = stmt.query([object_id_hex])?;

        if let Some(row) = rows.next()? {
            let tx_hex: String = row.get(0)?;
            Ok(Some(tx_hex))
        } else {
            Ok(None)
        }
    }
}

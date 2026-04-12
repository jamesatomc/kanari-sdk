// crates/kanari-types/src/effects.rs
use crate::digest::{ObjectDigest, TransactionDigest};
use move_core_types::account_address::AccountAddress;
use serde::{Deserialize, Serialize};

/// อ้างอิงถึง Object ที่ระบุตัวตนแบบเจาะจง (ID + Version + Hash)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectRef {
    pub object_id: AccountAddress,
    pub version: u64,
    pub digest: ObjectDigest,
}

/// สรุปค่าใช้จ่าย Gas ที่แบ่งสัดส่วนชัดเจนสำหรับ Tokenomics
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GasCostSummary {
    pub computation_cost: u64, // ค่าประมวลผล CPU ของโหนด
    pub storage_cost: u64,     // ค่าเช่าพื้นที่เก็บข้อมูล (State)
    pub storage_rebate: u64,   // ค่า Gas ที่คืนให้ผู้ใช้เมื่อลบ Object ทิ้ง
}

impl GasCostSummary {
    /// คำนวณค่าใช้จ่ายสุทธิที่ผู้ใช้ต้องจ่ายจริง
    pub fn net_gas_usage(&self) -> u64 {
        (self.computation_cost + self.storage_cost).saturating_sub(self.storage_rebate)
    }
}

/// สถานะการทำงานของ Transaction
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionStatus {
    Success,
    Failure { error: String },
}

/// ใบเสร็จผลลัพธ์การทำงานของ Transaction (นำไปใช้สร้าง State Root)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionEffects {
    /// Transaction ที่ทำให้เกิด Effect นี้
    pub transaction_digest: TransactionDigest,
    /// สถานะการทำงานสำเร็จหรือไม่
    pub status: ExecutionStatus,
    /// สรุปการใช้ Gas
    pub gas_used: GasCostSummary,
    /// รายการ Object ที่ถูกสร้างใหม่
    pub created: Vec<ObjectRef>,
    /// รายการ Object ที่ถูกแก้ไข
    pub mutated: Vec<ObjectRef>,
    /// รายการ Object ที่ถูกลบ
    pub deleted: Vec<ObjectRef>,
}

impl TransactionEffects {
    /// สร้าง Effects เปล่าๆ สำหรับใช้ในตอนเริ่มต้นสร้าง
    pub fn new(tx_digest: TransactionDigest) -> Self {
        Self {
            transaction_digest: tx_digest,
            status: ExecutionStatus::Success,
            gas_used: GasCostSummary::default(),
            created: Vec::new(),
            mutated: Vec::new(),
            deleted: Vec::new(),
        }
    }
}

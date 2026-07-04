// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::object::{ObjectID, ObjectRef, ObjectVersion};
use anyhow::{Result, ensure};
use move_core_types::account_address::AccountAddress;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ObjectArg {
    ImmOrOwnedObject(ObjectRef),
    SharedObject { id: ObjectID, initial_shared_version: ObjectVersion, mutable: bool },
    Receiving(ObjectRef),
}

impl ObjectArg {
    pub fn object_id(&self) -> ObjectID { match self { Self::ImmOrOwnedObject(r)|Self::Receiving(r)=>r.object_id, Self::SharedObject{id,..}=>*id } }
    pub fn object_ref(&self) -> Option<ObjectRef> { match self { Self::ImmOrOwnedObject(r)|Self::Receiving(r)=>Some(*r), _=>None } }
    pub fn is_mutable(&self) -> bool { !matches!(self, Self::SharedObject{mutable:false,..}) }
    pub fn requires_consensus(&self) -> bool { matches!(self, Self::SharedObject{..}) }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CallArg { Pure(Vec<u8>), Object(ObjectArg) }

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MoveCall {
    pub package: ObjectID,
    pub module: String,
    pub function: String,
    pub type_args: Vec<String>,
    pub arguments: Vec<CallArg>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ObjectTransactionKind {
    MoveCall(MoveCall),
    Publish { modules: Vec<Vec<u8>>, dependencies: Vec<ObjectID> },
    Pay { coins: Vec<ObjectRef>, recipient: AccountAddress, amount: u64 },
    TransferObjects { objects: Vec<ObjectRef>, recipient: AccountAddress },
}

impl ObjectTransactionKind {
    pub fn object_arguments(&self) -> impl Iterator<Item=&ObjectArg> {
        let args=match self { Self::MoveCall(c)=>Some(c.arguments.as_slice()), _=>None };
        args.into_iter().flatten().filter_map(|a| match a { CallArg::Object(o)=>Some(o), _=>None })
    }
    pub fn direct_refs(&self) -> &[ObjectRef] { match self { Self::Pay{coins,..}=>coins, Self::TransferObjects{objects,..}=>objects, _=>&[] } }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum TransactionExpiration { #[default] None, Epoch(u64) }

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GasData { pub payment: Vec<ObjectRef>, pub owner: AccountAddress, pub price: u64, pub budget: u64 }

impl GasData {
    pub fn is_sponsored_for(&self, sender:AccountAddress)->bool { self.owner!=sender }
    pub fn validate(&self)->Result<()> {
        ensure!(self.budget>0,"Gas budget must be greater than zero");
        ensure!(!self.payment.is_empty(),"At least one gas object is required");
        let ids:BTreeSet<_>=self.payment.iter().map(|r|r.object_id).collect();
        ensure!(ids.len()==self.payment.len(),"Duplicate gas object");
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ObjectTransactionData {
    pub sender: AccountAddress,
    pub kind: ObjectTransactionKind,
    pub gas_data: GasData,
    #[serde(default)] pub expiration: TransactionExpiration,
}

impl ObjectTransactionData {
    pub fn new(sender:AccountAddress,kind:ObjectTransactionKind,gas_data:GasData,expiration:TransactionExpiration)->Result<Self>{let tx=Self{sender,kind,gas_data,expiration};tx.validate()?;Ok(tx)}
    pub fn input_objects(&self)->impl Iterator<Item=&ObjectArg>{self.kind.object_arguments()}
    pub fn owned_input_refs(&self)->impl Iterator<Item=ObjectRef>+'_ { self.input_objects().filter_map(ObjectArg::object_ref).chain(self.kind.direct_refs().iter().copied()) }
    pub fn mutable_input_ids(&self)->BTreeSet<ObjectID>{self.input_objects().filter(|o|o.is_mutable()).map(ObjectArg::object_id).chain(self.kind.direct_refs().iter().map(|r|r.object_id)).chain(self.gas_data.payment.iter().map(|r|r.object_id)).collect()}
    pub fn read_input_ids(&self)->BTreeSet<ObjectID>{self.input_objects().map(ObjectArg::object_id).chain(self.kind.direct_refs().iter().map(|r|r.object_id)).collect()}
    pub fn requires_consensus(&self)->bool{self.input_objects().any(ObjectArg::requires_consensus)}
    pub fn requires_sponsor_signature(&self)->bool{self.gas_data.is_sponsored_for(self.sender)}
    pub fn validate(&self)->Result<()> {
        self.gas_data.validate()?;
        let mut ids=BTreeSet::new();
        for id in self.read_input_ids(){ensure!(ids.insert(id),"Duplicate transaction input");}
        for gas in &self.gas_data.payment{ensure!(ids.insert(gas.object_id),"Gas object is also a regular input");}
        match &self.kind {
            ObjectTransactionKind::MoveCall(c)=>{ensure!(!c.module.is_empty()&&!c.function.is_empty(),"Invalid Move call");}
            ObjectTransactionKind::Publish{modules,..}=>{ensure!(!modules.is_empty()&&modules.iter().all(|m|!m.is_empty()),"Invalid publish");}
            ObjectTransactionKind::Pay{coins,amount,..}=>{ensure!(!coins.is_empty()&&*amount>0,"Invalid pay command");}
            ObjectTransactionKind::TransferObjects{objects,..}=>{ensure!(!objects.is_empty(),"Invalid transfer command");}
        }
        Ok(())
    }
}

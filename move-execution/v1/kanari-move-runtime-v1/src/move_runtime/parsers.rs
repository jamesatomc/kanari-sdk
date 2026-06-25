// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

// Parser functions for Move VM changesets and events
use crate::changeset::ChangeSet;
use kanari_types::event::Event;
use kanari_types::object::{IDRecord, UIDRecord};
use log::debug;
use move_core_types::effects::Op as MoveOp;

impl super::MoveRuntime {
    /// Parse Move VM ChangeSet and extract state changes into Kanari ChangeSet
    /// This converts Move VM's canonical state changes into our domain model
    pub(crate) fn parse_move_changeset(
        &self,
        move_cs: &move_core_types::effects::ChangeSet,
        kanari_cs: &mut ChangeSet,
    ) {
        debug!(
            "[PARSER] parse_move_changeset: accounts={}, total_resources={}",
            move_cs.accounts().len(),
            move_cs
                .accounts()
                .values()
                .map(|a| a.resources().len())
                .sum::<usize>()
        );

        for (addr, account_changes) in move_cs.accounts() {
            for (module_name, op) in account_changes.modules() {
                match op {
                    MoveOp::New(bytes) | MoveOp::Modify(bytes) => {
                        kanari_cs.publish_module(*addr, module_name.to_string());
                        kanari_cs.write_move_module(*addr, module_name.to_string(), bytes.to_vec());
                    }
                    MoveOp::Delete => {
                        kanari_cs.delete_move_module(*addr, module_name.to_string());
                    }
                }
            }

            for (struct_tag, op) in account_changes.resources() {
                match op {
                    MoveOp::New(bytes) | MoveOp::Modify(bytes) => {
                        kanari_cs.write_move_resource(
                            *addr,
                            struct_tag.to_string(),
                            bytes.to_vec(),
                        );
                        // Extract UID from first 32 bytes if available (for Sui-style objects)
                        let uid_opt = if bytes.len() >= 32 {
                            let mut arr = [0u8; 32];
                            arr.copy_from_slice(&bytes[0..32]);
                            Some(UIDRecord::new(
                                move_core_types::account_address::AccountAddress::new(arr),
                            ))
                        } else {
                            None
                        };

                        // Create IDRecord for DEX/DeFi copyable ID tracking
                        let id_opt = if bytes.len() >= 32 {
                            let mut arr = [0u8; 32];
                            arr.copy_from_slice(&bytes[0..32]);
                            Some(IDRecord::new(
                                move_core_types::account_address::AccountAddress::new(arr),
                            ))
                        } else {
                            None
                        };

                        let final_object_id = if let Some(uid) = &uid_opt {
                            uid.address().to_hex_literal()
                        } else if let Some(id) = &id_opt {
                            id.address().to_hex_literal()
                        } else {
                            debug!(
                                "[PARSER] skipping resource without UID/ID: addr={} type={}",
                                addr.to_hex_literal(),
                                struct_tag
                            );
                            continue;
                        };

                        // Ordinary global resources are not objects. Only mirror a
                        // resource write when it is a writeback of an already-known object.
                        // Newly created objects arrive through the SavedObjects native extension.
                        if self.object_storage.get_object(&final_object_id).is_some() {
                            kanari_cs.add_created_object(
                                *addr,
                                format!("{}", struct_tag),
                                bytes.to_vec(),
                                0,
                                uid_opt,
                                id_opt,
                                Some(final_object_id),
                            );
                        } else {
                            debug!(
                                "[PARSER] skipping non-object global resource: addr={} type={}",
                                addr, struct_tag
                            );
                        }
                    }
                    MoveOp::Delete => {
                        kanari_cs.delete_move_resource(*addr, struct_tag.to_string());
                        debug!(
                            "[PARSER] recorded Move resource deletion: addr={} type={}",
                            addr.to_hex_literal(),
                            struct_tag
                        );
                    }
                }
            }
        }
    }

    /// Parse Move VM events and add to Kanari ChangeSet
    /// Events provide an audit trail of all state changes
    pub(crate) fn parse_move_events(
        &self,
        events: &[move_core_types::effects::Event],
        kanari_cs: &mut ChangeSet,
    ) {
        for event in events.iter() {
            let (key, sequence_number, type_tag, event_data) = event;
            let kanari_event = Event {
                key: key.clone(),
                sequence_number: *sequence_number,
                type_tag: format!("{}", type_tag),
                event_data: event_data.clone(),
            };
            kanari_cs.add_event(kanari_event);
        }
    }
}

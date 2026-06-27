// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::changeset::ChangeSet;
use kanari_types::event::Event;
use log::debug;
use move_core_types::account_address::AccountAddress;
use move_core_types::effects::Op as MoveOp;

impl super::MoveRuntime {
    fn object_id_from_resource_bytes(bytes: &[u8]) -> Option<String> {
        if bytes.len() < AccountAddress::LENGTH {
            return None;
        }

        let mut arr = [0u8; AccountAddress::LENGTH];
        arr.copy_from_slice(&bytes[..AccountAddress::LENGTH]);
        Some(AccountAddress::new(arr).to_hex_literal())
    }

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
                if matches!(op, MoveOp::New(_) | MoveOp::Modify(_)) {
                    kanari_cs.publish_module(*addr, module_name.to_string());
                }
            }

            for (struct_tag, op) in account_changes.resources() {
                match op {
                    MoveOp::New(bytes) | MoveOp::Modify(bytes) => {
                        let Some(object_id) = Self::object_id_from_resource_bytes(bytes) else {
                            debug!(
                                "[PARSER] skipping resource without UID/ID: addr={} type={}",
                                addr.to_hex_literal(),
                                struct_tag
                            );
                            continue;
                        };

                        let created = Self::build_created_object(
                            *addr,
                            &object_id,
                            &struct_tag.to_string(),
                            bytes.to_vec(),
                            0,
                        );

                        kanari_cs
                            .created_objects
                            .retain(|(existing_id, _)| existing_id != &object_id);
                        kanari_cs.created_objects.push((object_id, created));
                    }
                    MoveOp::Delete => {
                        debug!(
                            "[PARSER] skipping delete without concrete object id: addr={} type={}",
                            addr.to_hex_literal(),
                            struct_tag
                        );
                    }
                }
            }
        }
    }

    pub(crate) fn parse_move_events(
        &self,
        events: &[move_core_types::effects::Event],
        kanari_cs: &mut ChangeSet,
    ) {
        for (key, sequence_number, type_tag, event_data) in events {
            kanari_cs.add_event(Event {
                key: key.clone(),
                sequence_number: *sequence_number,
                type_tag: type_tag.to_string(),
                event_data: event_data.clone(),
            });
        }
    }
}

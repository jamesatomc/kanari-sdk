// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::changeset::ChangeSet;
use crate::common::ids::object_id_from_bytes;
use kanari_types::event::Event;
use log::debug;
use move_core_types::effects::Op as MoveOp;

impl super::MoveRuntime {
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
                let key = format!(
                    "module:{}:{}",
                    addr.to_hex_literal(),
                    module_name.as_str()
                )
                .into_bytes();
                match op {
                    MoveOp::New(bytes) | MoveOp::Modify(bytes) => {
                        kanari_cs.publish_module(*addr, module_name.to_string());
                        kanari_cs.record_move_write(key, Some(bytes.to_vec()));
                    }
                    MoveOp::Delete => kanari_cs.record_move_write(key, None),
                }
            }

            for (struct_tag, op) in account_changes.resources() {
                let resource_key =
                    format!("resource:{}:{}", addr.to_hex_literal(), struct_tag).into_bytes();
                match op {
                    MoveOp::New(bytes) | MoveOp::Modify(bytes) => {
                        kanari_cs.record_move_write(resource_key, Some(bytes.to_vec()));

                        let Some(object_id) = object_id_from_bytes(bytes) else {
                            debug!(
                                "[PARSER] resource has no UID/ID: addr={} type={}",
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
                        kanari_cs.record_move_write(resource_key, None);
                        debug!(
                            "[PARSER] resource delete recorded: addr={} type={}",
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

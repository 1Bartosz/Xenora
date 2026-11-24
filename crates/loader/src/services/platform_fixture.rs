// SPDX-License-Identifier: GPL-3.0-or-later

//! Process-local platform services for original guest fixtures.

use super::{ERROR_INVALID_PARAMETER, ERROR_NOACCESS, ERROR_NOT_ENOUGH_MEMORY, ServicesState};
use crate::imports::ImportRegistry;
use std::collections::BTreeMap;
use std::sync::Arc;

const MODULE: &str = "xenora_fixture.dll";
const FIXTURE_APP_ID: u64 = 0x5845_4e4f_5241_0001;
const ERROR_NOT_FOUND: u32 = 1168;
const ERROR_INVALID_STATE: u32 = 5023;
const MAX_SETTINGS: usize = 32;

#[derive(Default)]
pub(super) struct PlatformFixture {
    active: bool,
    settings: BTreeMap<u32, u64>,
}

pub(super) fn register(registry: &mut ImportRegistry, state: &Arc<ServicesState>) {
    for (symbol, active) in [
        ("OpenFixturePlatform", true),
        ("CloseFixturePlatform", false),
    ] {
        let state = Arc::clone(state);
        registry.register(MODULE, symbol, move |_, cpu, _| {
            let mut platform = state.platform_fixture.lock().expect("fixture platform");
            if platform.active == active {
                return state.fail(cpu.gs_base, ERROR_INVALID_STATE, 0);
            }
            platform.active = active;
            Ok(1)
        });
    }

    let identity = Arc::clone(state);
    registry.register(MODULE, "GetFixtureApplicationId", move |_, cpu, _| {
        let platform = identity.platform_fixture.lock().expect("fixture platform");
        if !platform.active {
            return identity.fail(cpu.gs_base, ERROR_INVALID_STATE, 0);
        }
        Ok(FIXTURE_APP_ID)
    });

    let set = Arc::clone(state);
    registry.register(MODULE, "SetFixtureSetting", move |args, cpu, _| {
        let mut platform = set.platform_fixture.lock().expect("fixture platform");
        if !platform.active {
            return set.fail(cpu.gs_base, ERROR_INVALID_STATE, 0);
        }
        let key = args.rcx as u32;
        if key == 0 {
            return set.fail(cpu.gs_base, ERROR_INVALID_PARAMETER, 0);
        }
        if platform.settings.len() >= MAX_SETTINGS && !platform.settings.contains_key(&key) {
            return set.fail(cpu.gs_base, ERROR_NOT_ENOUGH_MEMORY, 0);
        }
        platform.settings.insert(key, args.rdx);
        Ok(1)
    });

    let get = Arc::clone(state);
    registry.register(MODULE, "GetFixtureSetting", move |args, cpu, memory| {
        let platform = get.platform_fixture.lock().expect("fixture platform");
        if !platform.active {
            return get.fail(cpu.gs_base, ERROR_INVALID_STATE, 0);
        }
        let key = args.rcx as u32;
        if key == 0 {
            return get.fail(cpu.gs_base, ERROR_INVALID_PARAMETER, 0);
        }
        let Some(&value) = platform.settings.get(&key) else {
            return get.fail(cpu.gs_base, ERROR_NOT_FOUND, 0);
        };
        if args.rdx == 0 || memory.write_u64(args.rdx, value).is_err() {
            return get.fail(cpu.gs_base, ERROR_NOACCESS, 0);
        }
        Ok(1)
    });
}

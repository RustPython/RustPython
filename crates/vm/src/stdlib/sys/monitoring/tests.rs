use super::{
    CodeMonitoringEntry, MonitoringCodeIdentity, MonitoringEvent, MonitoringLayerSite,
    MonitoringState, PyRc, UNGROUPED_EVENTS_COUNT, identity_key,
};
use crate::common::lock::PyMutex;
use rustpython_compiler_core::bytecode::Instruction;

#[test]
fn scalar_layer_continuations_preserve_operation_identity() {
    let mut site = MonitoringLayerSite::new(Instruction::ReturnValue, 0, true, true);
    let empty = [0; UNGROUPED_EVENTS_COUNT];
    let mut all = empty;
    all[MonitoringEvent::PyReturn as usize] = 1;
    all[MonitoringEvent::Line as usize] = 1;
    all[MonitoringEvent::Instruction as usize] = 1;
    let mut line = empty;
    line[MonitoringEvent::Line as usize] = 1;
    let mut instruction = empty;
    instruction[MonitoringEvent::Instruction as usize] = 1;

    site.transition(&empty, &all);
    assert_eq!(u8::from(site.top), u8::from(Instruction::InstrumentedLine));
    assert_eq!(
        u8::from(site.instruction_next),
        u8::from(Instruction::InstrumentedReturnValue)
    );

    site.transition(&line, &empty);
    assert_eq!(
        u8::from(site.top),
        u8::from(Instruction::InstrumentedInstruction)
    );
    assert_eq!(
        u8::from(site.line_next),
        u8::from(Instruction::InstrumentedReturnValue)
    );
    site.transition(&empty, &line);
    assert_eq!(
        u8::from(site.line_next),
        u8::from(Instruction::InstrumentedInstruction)
    );

    site.transition(&instruction, &empty);
    assert_eq!(
        u8::from(site.line_next),
        u8::from(Instruction::InstrumentedReturnValue)
    );
    site.transition(&empty, &instruction);
    let selected = site.instruction_next;
    site.transition(&all, &empty);
    assert_eq!(u8::from(site.top), u8::from(Instruction::ReturnValue));
    assert_eq!(
        u8::from(selected),
        u8::from(Instruction::InstrumentedReturnValue)
    );
    site.transition(&empty, &all);
    site.transition(&all, &all);
    assert_eq!(u8::from(site.top), u8::from(Instruction::InstrumentedLine));
    assert_eq!(
        u8::from(site.line_next),
        u8::from(Instruction::InstrumentedInstruction)
    );
    assert_eq!(
        u8::from(site.instruction_next),
        u8::from(Instruction::InstrumentedReturnValue)
    );
    assert_eq!(
        (site.direct_tools, site.line_tools, site.instruction_tools),
        (1, 1, 1)
    );
    // Initial disabled-set insertion order must not choose saved continuations.
    let identity = PyRc::new(MonitoringCodeIdentity);
    let mut forward = CodeMonitoringEntry::new(&identity);
    let mut reverse = CodeMonitoringEntry::new(&identity);
    let disabled = [
        MonitoringEvent::Line as usize,
        MonitoringEvent::PyReturn as usize,
    ];
    for event in disabled {
        forward.disabled.insert((0, event, 0));
    }
    for event in disabled.into_iter().rev() {
        reverse.disabled.insert((0, event, 0));
    }
    let forward_mask = forward.disabled_tools(0);
    let reverse_mask = reverse.disabled_tools(0);
    assert_eq!(forward_mask, reverse_mask);
    let mut forward_site = site;
    let mut reverse_site = site;
    forward_site.transition(&forward_mask, &empty);
    reverse_site.transition(&reverse_mask, &empty);
    let snapshot = |site: MonitoringLayerSite| {
        (
            u8::from(site.top),
            u8::from(site.line_next),
            u8::from(site.instruction_next),
            site.direct_tools,
            site.line_tools,
            site.instruction_tools,
        )
    };
    assert_eq!(snapshot(forward_site), snapshot(reverse_site));
    assert_eq!(
        u8::from(forward_site.line_next),
        u8::from(Instruction::ReturnValue)
    );
}

#[test]
fn native_identity_weak_witness_outlives_only_allocation() {
    let first = PyRc::new(MonitoringCodeIdentity);
    let second = PyRc::new(MonitoringCodeIdentity);
    let first_key = identity_key(&first);
    assert_ne!(first_key, identity_key(&second));
    let alias = first.clone();
    assert_eq!(first_key, identity_key(&alias));
    let witness = PyRc::downgrade(&first);
    drop(first);
    assert_eq!(witness.strong_count(), 1);
    drop(alias);
    assert_eq!(witness.strong_count(), 0);
    let third = PyRc::new(MonitoringCodeIdentity);
    assert_ne!(first_key, identity_key(&third));
    assert_ne!(identity_key(&second), identity_key(&third));
}

#[test]
fn witnessed_entries_keep_live_state_and_retire_dead_allocations() {
    let first = PyRc::new(MonitoringCodeIdentity);
    let second = PyRc::new(MonitoringCodeIdentity);
    let first_key = identity_key(&first);
    let second_key = identity_key(&second);
    let state = PyMutex::new(MonitoringState::default());
    {
        let mut state = state.lock();
        state.ensure_code(&first);
        state.ensure_code(&second);
        state.codes.get_mut(&first_key).unwrap().local_events[0] = MonitoringEvent::Line.mask();
        state.synchronize_layers(false);
        state
            .codes
            .get_mut(&first_key)
            .unwrap()
            .disable_site(0, MonitoringEvent::Line as usize, 0);
        assert!(state.disabled(first_key, 0, MonitoringEvent::Line as usize, 0));
        state.codes.get_mut(&first_key).unwrap().local_events[0] = 0;
        state.synchronize_layers(false);
        assert!(state.sweep_dead_codes().is_empty());
        assert!(state.codes.contains_key(&first_key));
        state.codes.get_mut(&first_key).unwrap().local_events[0] = MonitoringEvent::Line.mask();
        state.synchronize_layers(false);
        assert!(!state.disabled(first_key, 0, MonitoringEvent::Line as usize, 0));
        assert_eq!(
            state.local_events(first_key, 0),
            MonitoringEvent::Line.mask()
        );
    }
    drop(first);
    let retired = {
        let mut state = state.lock();
        let retired = state.sweep_dead_codes();
        assert!(!state.codes.contains_key(&first_key));
        assert!(state.codes.contains_key(&second_key));
        assert_eq!(state.combined_events(), 0);
        retired
    };
    assert_eq!(retired.len(), 1);
    assert_eq!(retired[0].identity.strong_count(), 0);
    drop(retired);
}

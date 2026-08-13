use zene_tools::PlanModeState;

#[test]
fn plan_mode_blocks_write() {
    let mut state = PlanModeState::default();
    state.enter();
    assert!(!state.is_tool_allowed("Write"));
    assert!(!state.is_tool_allowed("Edit"));
    assert!(!state.is_tool_allowed("Bash"));
    assert!(!state.is_tool_allowed("Task"));
    assert!(state.is_tool_allowed("Read"));
    assert!(state.is_tool_allowed("RepoMap"));
}

#[test]
fn exit_restores_write_allowed() {
    let mut state = PlanModeState::default();
    state.enter();
    assert!(state.is_active());
    assert!(!state.is_tool_allowed("Write"));
    state.exit();
    assert!(!state.is_active());
    assert!(state.is_tool_allowed("Write"));
}

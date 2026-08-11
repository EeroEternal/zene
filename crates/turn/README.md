# zene-turn

Turn state (`TurnId`, `SteerBuffer`) and the multi-step [`TurnEngine`](src/turn_loop.rs).

`TurnEngine` depends on explicit session, model, tool, and event ports. This keeps
turn orchestration independent from `zene-core`, ACP, Cloud, and provider
implementations. `LegacyTurnPorts` adapts the pre-Wave 5 [`TurnRuntime`] API,
and `run_turn_loop` remains as a backward-compatible string-returning facade.

## License

MIT

# zene-turn

Turn state (`TurnId`, `SteerBuffer`) and the multi-step turn loop via [`TurnRuntime`](src/turn_loop.rs).

Runtime implements `TurnRuntime`; `run_turn_loop` orchestrates prepare → step → tools → steer without coupling to `zene-core`.

## License

MIT

# zene-permission

Tool permission modes, rules, and `PermissionGate` for [Zene](https://github.com/ParaTensor/zene) agents.

Policy (`PermissionGate::evaluate`) is a pure allow/deny/ask decision. User
interaction goes through [`ApprovalBroker`](src/broker.rs) so ACP, Cloud, and
tests can inject waiters without holding a sync mutex.

## License

MIT

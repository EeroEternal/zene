# zene-session

Session persistence for [Zene](https://github.com/ParaTensor/zene) agents: append-only event-tree transcript storage, compaction history, checkpoints, and todos.

## Features

- `SessionRecord` event log for messages, compactions, branch summaries, labels, and custom facts
- Event-backed context, replay, and export projections; `messages` is a compatibility cache
- Compaction checkpoints for `/rewind`
- JSON session files under `~/.zene/sessions`
- `AgentRecordWriter` for structured run logs

## License

MIT

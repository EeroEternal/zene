/**
 * Node-specific entry point for `@zene/agent`. Currently exports the Node
 * implementation of `defineCommand`.
 *
 * Import platform-agnostic types (`FlueContext`, `Command`, etc.) from
 * `@zene/agent/client`.
 */
export { defineCommand, type CommandOptions } from './define-command.ts';

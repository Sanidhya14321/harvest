Inspect or control a program through a host-configured native debug adapter.
Choose an adapter and action; supply program/args for launch or structured arguments for attach.
Actions include launch, attach, set_breakpoint, remove_breakpoint, continue, step_over, step_in, step_out, pause, threads, stack_trace, scopes, variables, evaluate, read_memory, write_memory, disassemble, modules, loaded_sources, custom_request, output, terminate, disconnect, sessions.
Thread-specific operations require thread_id. Frame, variable and memory references come from previous adapter results.
Debugger commands require host approval of the final structured request. Reverse terminal/process launching is unavailable. The program is a path, never a shell command.

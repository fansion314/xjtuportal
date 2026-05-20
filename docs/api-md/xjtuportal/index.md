# xjtuportal

High-level orchestration for unattended campus portal login.

This crate is the reusable core behind the CLI. It resolves TOML
configuration into login targets, checks captive-portal state, performs v3
encrypted login, lists sessions, and optionally logs out one existing device
before retrying. The default behavior remains unattended login; interactive
shell-style flows should not be reintroduced here.

## Modules

### [`xjtuportal`](xjtuportal.md)

*1 enum, 10 functions, 3 structs, 6 modules*

### [`config`](config.md)

*1 function, 9 structs*

### [`crypto`](crypto.md)

*4 functions*

### [`error`](error.md)

*1 enum, 1 type alias*

### [`interface`](interface.md)

*3 functions*

### [`portal`](portal.md)

*2 enums, 2 functions, 2 structs*

### [`session`](session.md)

*2 functions, 3 structs*


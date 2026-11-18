09-tui describes themes as TOML tables that assign a colour to each token; no step builds a user's theme file, and step 24's hot reload reloads `tui.theme`, the name of a built-in theme.

- Area: cineris-tui
- Owner: none yet

| Item | Who | When |
|---|---|---|
| **A theme is a built-in one, chosen by name.** [09-tui.md](../architecture/09-tui.md) says themes "are TOML tables that assign a colour to each token". No step has built that: `tui.theme` names `auto`, `dark` or `light`, and step 24's hot reload ([steps/24/answered.md](../steps/24/answered.md), point 5) takes "a theme file" to mean the configuration file that holds `tui.theme`, read again when it changes. **Recommendation:** a `[themes.<name>]` table in the user's configuration (user-only, `at once`), each key a token of `cineris_tui::theme::Token` and each value a colour, read by the same reload; a token it leaves out keeps the built-in theme's colour, and a name it does not know is said once. | The owner, then a step of the interface | Before the first release, if users are to bring their own colours |

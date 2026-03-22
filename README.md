# khora

Web app QA automation for agents. Navigate pages, find elements, click buttons, type text, take screenshots — all from the command line via Chrome DevTools Protocol.

Cross-platform. Built for CI/CD pipelines and agent workflows where you need to verify a web app actually works, not just that it builds.

## Install

```
brew install simonspoon/tap/khora
```

Or download from [Releases](https://github.com/simonspoon/khora/releases).

## Quick start

```bash
# Launch Chrome (headless by default)
khora launch
# Session: abc123

# Navigate to a page
khora navigate abc123 https://example.com

# Find elements
khora find abc123 "h1"
khora find abc123 "button.submit"

# Interact
khora click abc123 "button.submit"
khora type abc123 "input[name=email]" "user@example.com"

# Verify content
khora text abc123 "h1"
khora attribute abc123 "a" "href"
khora wait-for abc123 ".success-message"

# Screenshot
khora screenshot abc123 -o result.png

# Execute JavaScript
khora eval abc123 "document.title"

# Clean up
khora kill abc123
```

Use `--visible` to see the browser window:

```bash
khora launch --visible
```

## Commands

| Command | Description |
|---------|-------------|
| `launch` | Start Chrome (headless by default, `--visible` for headed) |
| `navigate <session> <url>` | Go to a URL |
| `find <session> <selector>` | Find elements by CSS selector |
| `click <session> <selector>` | Click an element |
| `type <session> <selector> <text>` | Type text into an element |
| `screenshot <session>` | Capture full page screenshot |
| `text <session> <selector>` | Get text content of matching elements |
| `attribute <session> <selector> <attr>` | Get attribute value |
| `wait-for <session> <selector>` | Wait for element to appear |
| `wait-gone <session> <selector>` | Wait for element to disappear |
| `console <session>` | Read console messages |
| `eval <session> <js>` | Execute JavaScript, return result |
| `kill <session>` | Close browser and clean up |
| `status [session]` | Check session status (or list all) |

## Output

All commands support `--format json` for structured output. Use `KHORA_FORMAT=json` to default to JSON.

Default timeout is 5000ms, override with `--timeout` or `KHORA_TIMEOUT`.

## Session management

Sessions persist across CLI invocations. `launch` starts Chrome and saves a session file to `~/.khora/sessions/`. Subsequent commands reconnect using the session ID. `kill` closes the browser and removes the session file.

Multiple concurrent sessions are supported — each gets a unique ID.

## Requirements

- Chrome or Chromium installed (auto-detected per platform)
- Or set `CHROME_PATH` to a custom Chrome binary

## License

MIT

# Usage Guide

## Installation

### Homebrew

```bash
brew install simonspoon/tap/khora
```

### From source

```bash
git clone https://github.com/simonspoon/khora.git
cd khora
cargo install --path crates/khora-cli
```

## Chrome setup

Khora auto-detects Chrome/Chromium:

| Platform | Search locations |
|----------|-----------------|
| macOS | `/Applications/Google Chrome.app`, Chromium, Brave, Edge |
| Linux | `/usr/bin/google-chrome`, `/usr/bin/chromium`, snap |
| Windows | Program Files (Chrome, Edge, Brave) |

Override with `CHROME_PATH=/path/to/chrome`.

## Session workflow

Every interaction starts with `launch` and ends with `kill`:

```bash
SESSION=$(khora --format json launch | jq -r .id)
khora navigate "$SESSION" "https://your-app.com"
# ... interact and verify ...
khora kill "$SESSION"
```

### Headed mode

```bash
khora launch --visible
```

### Multiple sessions

```bash
SESSION1=$(khora --format json launch | jq -r .id)
SESSION2=$(khora --format json launch | jq -r .id)
khora navigate "$SESSION1" "https://app1.com"
khora navigate "$SESSION2" "https://app2.com"
khora status        # lists all sessions
khora kill --all    # kill every active session at once
```

## Common patterns

### Verify page content

```bash
khora navigate "$S" "https://app.com/dashboard"
khora text "$S" "h1"  # Check heading
khora wait-for "$S" ".data-loaded"  # Wait for dynamic content
khora text "$S" ".user-count"  # Read a value
```

### Form interaction

```bash
khora navigate "$S" "https://app.com/login"
khora type "$S" "input[name=email]" "test@example.com"
khora type "$S" "input[name=password]" "password123"
khora click "$S" "button[type=submit]"
khora wait-for "$S" ".dashboard"
```

### Screenshot comparison

```bash
khora navigate "$S" "https://app.com"
khora screenshot "$S" -o before.png
# ... make changes ...
khora screenshot "$S" -o after.png
```

### JavaScript evaluation

```bash
# Get computed values
khora eval "$S" "window.innerWidth"
khora eval "$S" "document.querySelectorAll('li').length"

# Check application state
khora eval "$S" "JSON.stringify(window.__APP_STATE__)"
```

### Element inspection

```bash
# Find all matching elements (JSON for parsing)
khora --format json find "$S" "button"

# Get specific attributes
khora attribute "$S" "img.logo" "src"
khora attribute "$S" "a.nav-link" "href"
```

### Wait patterns

```bash
# Wait for element to appear (default 5s timeout)
khora wait-for "$S" ".success-banner"

# Custom timeout
khora wait-for "$S" ".slow-content" --timeout 15000

# Wait for element to disappear
khora wait-gone "$S" ".loading-spinner"
```

## JSON output

All commands support `--format json`. Set `KHORA_FORMAT=json` for agent workflows:

```bash
export KHORA_FORMAT=json
khora find "$S" "button"
# Returns structured JSON with tag_name, text, bounding_box, etc.
```

## Timeouts

Default: 5000ms. Override per-command with `--timeout` or globally with `KHORA_TIMEOUT`:

```bash
khora --timeout 10000 wait-for "$S" ".slow-element"
# or
export KHORA_TIMEOUT=10000
```

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | General error |
| 2 | Chrome not found |
| 3 | Timeout |
| 4 | Session not found or dead |

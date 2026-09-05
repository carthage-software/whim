# Editor Setup

Whim includes a language server in the `whim` command. Keep `whim` on the
editor's `PATH` so the editor can run:

```console
whim language-server
```

The language server provides highlighting, basic completion, snippets, formatting,
folding, selection ranges, and occurrence highlighting.

## Zed

Install **Whim Language Support** (`whim-lang`) from Zed's Extensions page:

1. Open Zed's Extensions page.
2. Search for `whim-lang` or **Whim Language Support**.
3. Select **Install**.

The extension's source is available in the [Whim Zed repository](https://github.com/carthage-software/whim-zed).

If you use version 0.1.0, add this to Zed's settings for full highlighting:

```json
{
  "languages": {
    "Whim": {
      "semantic_tokens": "full"
    }
  }
}
```

Later versions use the full Tree-sitter Whim grammar and do not need this
setting.

## Helix

Add this to `~/.config/helix/languages.toml`:

```toml
use-grammars = { only = ["whim"] }

[[language]]
name = "whim"
language-id = "whim"
scope = "source.whim"
file-types = ["whim"]
shebangs = ["whim"]
roots = ["whim.toml"]
comment-tokens = ["//"]
block-comment-tokens = { start = "/*", end = "*/" }
indent = { tab-width = 2, unit = "  " }
language-servers = ["whim"]
auto-format = true
grammar = "whim"

[language-server.whim]
command = "whim"
args = ["language-server"]

[[grammar]]
name = "whim"
source = { git = "https://github.com/carthage-software/tree-sitter-whim", rev = "99e550efd095bf0b0f782e096b0ed6136bebaf47" }
```

`use-grammars` keeps the grammar commands limited to Whim. If the file already sets it, add `"whim"` to its existing list instead.

Install the full highlighting query from Tree-sitter Whim:

```console
mkdir -p ~/.config/helix/runtime/queries/whim
curl --fail --location \
  https://raw.githubusercontent.com/carthage-software/tree-sitter-whim/99e550efd095bf0b0f782e096b0ed6136bebaf47/queries/highlights.scm \
  --output ~/.config/helix/runtime/queries/whim/highlights.scm
```

Fetch and build the grammar, then check the setup:

```console
hx --grammar fetch
hx --grammar build
hx --health whim
```

Remove `auto-format = true` if you do not want Helix to format files when you save them.

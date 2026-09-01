# Editor Setup

Whim includes a language server in the `whim` command. Keep `whim` on the
editor's `PATH` so the editor can run:

```console
whim language-server
```

The language server provides highlighting, basic completion, snippets, formatting,
folding, selection ranges, and occurrence highlighting.

## Zed

Zed has not yet merged the [Whim extension](https://github.com/zed-industries/extensions/pull/7441).
Until it does, install the extension from its [source repository](https://github.com/carthage-software/whim-zed):

1. Install Rust with `rustup`. Zed uses it to build the development extension.
2. Clone the extension:

   ```console
   git clone https://github.com/carthage-software/whim-zed.git
   ```

3. Open Zed's Extensions page.
4. Select **Install Dev Extension** and choose the cloned directory.

Zed turns language-server semantic tokens off by default. Add this to Zed's
settings for full highlighting:

```json
{
  "languages": {
    "Whim": {
      "semantic_tokens": "full"
    }
  }
}
```

The bundled Tree-sitter grammar still highlights Whim keywords without this setting.

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
source = { git = "https://github.com/carthage-software/tree-sitter-whim", rev = "7582557f1d87d7af4957b98b351ad0a1c42bd7cd" }
```

`use-grammars` keeps the grammar commands limited to Whim. If the file already sets it, add `"whim"` to its existing list instead.

Install the keyword-highlighting query:

```console
mkdir -p ~/.config/helix/runtime/queries/whim
printf '%s\n' '(keyword) @keyword' > ~/.config/helix/runtime/queries/whim/highlights.scm
```

Fetch and build the grammar, then check the setup:

```console
hx --grammar fetch
hx --grammar build
hx --health whim
```

Remove `auto-format = true` if you do not want Helix to format files when you save them.

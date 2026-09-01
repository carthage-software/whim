# Git Dependencies

Whim uses Git repositories as package identities and SemVer tags as releases.
It has no registry, package names, global cache, install scripts, features, or
path-only package form.

Every package command stores data under the current project. `whim run` knows
nothing about this system.

Whim also maintains [official packages](official-packages.md) in separate Git
repositories. They use the same commands and file layout as any other package.

## Manifest

The root `whim.toml` may contain:

```toml
manifest-version = 1

[package]
repository = "https://github.com/acme/application"
homepage = "https://acme.example"
author = "Acme"
description = "An example application."
license = "MIT"
sponsor = "https://github.com/sponsors/acme"

[requirements]
whim = "^0.1"

[autoload.namespaces]
"App\\" = "src/"

[dependencies]
"git+https://github.com/acme/router.git" = "^1.2"

[dev-dependencies]
"git+ssh://git@github.com/acme/testing.git" = { version = "~2.0" }

[conflicts]
"git+https://github.com/acme/old-router.git" = "*"

[suggests]
"git+https://github.com/acme/profiler.git" = "^1"

[overrides]
"git+https://github.com/acme/router.git" = "git+https://github.com/acme/router-fork.git"

[format]
include = ["**/*.whim"]
exclude = ["src/generated/**"]
print_width = 80
tab_width = 2
use_tabs = false
end_of_line = "lf"

[runtime]
optimizations = "on"
call-depth = 10000
cycle-threshold = 10001
full-trace = false
```

Every manifest starts with `manifest-version = 1`. Unknown fields are errors.

### Runtime settings

`[runtime]` controls program execution. `optimizations` accepts `"on"` or
`"off"`. `call-depth` sets the frame limit. `cycle-threshold` sets the cycle
collector's root limit. `full-trace` keeps `TraceBoundary` frames in traces.

All four fields are optional. `WHIM_OPTIMIZATIONS`, `WHIM_CALL_DEPTH`,
`WHIM_CYCLE_THRESHOLD`, and `WHIM_FULL_TRACE` override them for one command.
The boolean environment value accepts `true` or `false`.

### Package details

`[package]` is optional. `repository`, `homepage`, `author`, `description`,
`license`, and `sponsor` describe the project. They do not identify it or alter
resolution.

`license` must be a valid SPDX expression. With no license, Whim treats the
package as proprietary when it warns about incompatible licenses. `sponsor`
must be an HTTP or HTTPS URL without credentials.

Whim warns when a dependency has no license. It also warns when a dependency
requires a copyleft license but the root license has no copyleft choice. This
check is a warning, not legal advice or a full license proof.

### Engine requirement

`[requirements].whim` limits the Whim versions that may use the package. A
missing requirement accepts any version. The root and every selected package
must accept the running Whim version.

### Source groups

`[dependencies]` is part of the running application. `[dev-dependencies]` is
for development. One source cannot appear in both. Whim ignores development
dependencies declared by installed packages.

Each value is a Cargo-style SemVer requirement. The short and table forms are
equal:

```toml
[dependencies]
"git+https://github.com/acme/a.git" = "^1.2"
"git+https://github.com/acme/b.git" = { version = ">=2, <3" }
```

Whim accepts exact, caret, tilde, wildcard, and comma-joined comparisons. It
does not accept `||`. The requirement must permit a pre-release before the
resolver can select it.

`[conflicts]` names versions that cannot share a graph with this package.
`[suggests]` lists useful packages that Whim does not install on its own.

`[overrides]` belongs only in the root manifest. It tells Whim to read releases
and code from a replacement repository while keeping the original repository
as the graph identity. The replacement must have tags that satisfy the
original requirements.

### Namespace maps

Each `[autoload.namespaces]` key ends in `\`. Its value is a relative directory
inside the repository. The empty prefix and `Whim\` are forbidden. When
prefixes overlap, the longest prefix wins. Two selected packages may not export
the same exact prefix.

## Git sources

Whim accepts:

- `git+https://`
- `git+ssh://`
- `git+file://`
- normal HTTPS URLs and SCP-style SSH input accepted by package commands

The manager normalizes command input before storing identity. It rejects plain
HTTP, `git://`, relative paths, URL queries, fragments, passwords, tokens, and
Git transport helpers. HTTPS, SSH, and local file URLs remain different
identities even when they point to the same repository.

Each release tag must be `1.2.3` or `v1.2.3`. If both names point to one commit,
they are one release. If they point to different commits, that version is
ambiguous and cannot resolve. Whim ignores non-SemVer tags.

## Resolution

Whim selects one version for each normalized source. All paths to that source
must accept the same version. An override changes the Git repository that
supplies versions and code; it does not change the source key in the graph.

The resolver prefers a version already present in `whim.lock` when that version
still meets all rules. For an unlocked source, it chooses the highest matching
release. It selects a pre-release only when a requirement permits that exact
pre-release line.

Each selected manifest must accept the running Whim version. Whim rejects a
self-dependency, a dependency cycle, a selected conflict, or two constraints
with no common release. The error shows the sources and requirements that led
to the conflict.

The resolver reads only runtime dependencies from installed packages. It also
reads their Whim requirement, namespace maps, conflicts, suggestions, license,
and sponsor link. It rejects overrides in an installed package.

## Commands

### init

`whim init` creates a project in the current directory. It writes `whim.toml`,
`src/main.whim`, and `tests/main.whim`, then starts a Git repository unless the
directory already belongs to one. It also writes `.gitignore` and
`.gitattributes` rules for dependency and archive files.

```console
whim init
whim init --no-git
```

`--no-git` skips Git and both Git files. Initialization preserves existing
source files and existing Git rules. It never overwrites an existing manifest.

### add and remove

```console
whim add https://github.com/acme/router.git --version '^1.2'
whim add git@github.com:acme/testing.git --dev
whim remove https://github.com/acme/router.git
```

Without `--version`, `add` chooses the latest stable release and writes a caret
requirement. Adding an existing source changes its requirement. Moving it
between runtime and development requires removing it first.

Both commands resolve and stage the whole new state before replacing tracked
files.

### install

```console
whim install
whim install --no-dev
```

With no lock, `install` resolves the graph and creates one. With a current lock,
it installs those exact commits and does not choose newer tags. It fails when
resolution fields in the manifest no longer match the lock.

`--no-dev` leaves the development closure and its namespaces out of the vendor
tree and loader. A warm install reuses checked local data and need not contact
the remote.

### update

```console
whim update
whim update https://github.com/acme/router.git
```

With no source, `update` may move the whole graph within its requirements. With
sources, it unlocks those identities and moves other packages only when their
constraints require it. A targeted update needs an existing lock.

If a locked version tag now points to another commit, update stops and reports
the old and new commit IDs.

### explain and inspect

`whim show SOURCE` prints details about one installed package. The report
includes its locked version, tag, commit, install path, package metadata,
namespace maps, Whim requirement, dependencies, development dependencies,
conflicts, and suggestions. The source may use any spelling that normalizes to
the locked Git identity.

`whim why SOURCE` prints the chain that requires a locked source and says
whether it is installed.

`whim why-not SOURCE --version RANGE` explains why that source and range cannot
join the current graph. The range defaults to `*`.

`whim suggestions` lists suggestions from the root project and installed
packages. Each entry names who suggested it. `whim fund` prints Whim's sponsor
link and the sponsor links in the installed graph.

## Lock and vendor tree

Commit `whim.lock`. It pins each normalized source, version, tag, commit, tree,
manifest hash, exported-file hash, dependencies, optional replacement source,
license, sponsor link, and suggestions. It stores no time, credential, or
incidental working-directory path. A configured `git+file://` source is an
absolute identity, so its path appears in the lock.

The lock also stores a hash of the root fields that affect resolution:

- the Whim requirement;
- namespace maps;
- runtime and development dependencies;
- conflicts;
- overrides.

Package details, format settings, suggestions, comments, and TOML order do not
make a root lock stale. For an installed package, the hash covers its Whim
requirement, namespace maps, runtime dependencies, and conflicts.

The lock parser rejects unknown fields, unsorted or repeated package entries,
malformed hashes, and references to missing packages.

Ignore `vendor/` unless the project wants to commit installed source for an
offline release. Its layout is:

```text
vendor/
  autoload.whim
  packages/<source-hash>/
  .whim/git/<source-hash>.git/
  .whim/stages/
  .whim/state.toml
  .whim/install.lock
```

Package directories use the full BLAKE3 hash of the normalized source. The
local bare Git repositories live only in this project. `install.lock` stops
two package commands from changing the project at once.

## Loading packages

Applications opt in:

```whim,norun
require_once!(directory!() . '/vendor/autoload.whim');
```

For `App\Model\User` mapped to `src/`, the loader tries:

1. `src/Model/User.whim`
2. a group file in `src/Model/`

The group file is `classes.whim`, `interfaces.whim`, `enums.whim`,
`functions.whim`, `types.whim`, or `constants.whim`, based on the requested
symbol kind.

The loader performs at most two file checks for one matched prefix. A file that
loads but does not define the requested symbol causes `UndefinedSymbolError`.

## Safety and transactions

Whim uses the system Git program without a shell. It works only with bare
repositories that it created. It does not run package hooks, scripts, filters,
submodules, or executables.

Remote Git commands stop after five minutes. Set
`WHIM_PACKAGE_NETWORK_TIMEOUT` to a positive number of seconds to change this
limit for one command.

The archive reader rejects absolute paths, parent traversal, links, devices,
FIFOs, excessive path lengths, and size limits. Whim hashes sorted file paths,
modes, lengths, and contents, then checks an existing vendor tree before reuse.

The fixed input limits are:

- 1 MiB for a manifest, lock, or installed-state file;
- 8,192 sources in one graph;
- 100,000 tags from one source;
- 100,000 files in one package;
- 4,096 bytes in one package path;
- 256 MiB in one file;
- 1 GiB of unpacked data in one package.

Whim rejects archive links, devices, and FIFOs rather than trying to copy them.
It keeps regular files, directories, and executable mode bits.

Package commands take a project lock. They prepare packages and the loader
under `vendor/.whim/stages/`, and keep backups and recovery data under
`vendor/.whim/`. They then swap the manifest, lock, vendor tree, and loader
together. The project root changes only during that final swap. The next
package command stops if it finds `vendor/.whim/transaction.pending`.

Inspect the manifest, lockfile, vendor tree, loader, state file, staging
directories, and backups before removing that marker. Whim does not recover an
interrupted swap on its own.

# Official Packages

The Whim project maintains packages in the
[Trifle group on Codeberg](https://codeberg.org/trifle). They live outside the
main Whim repository and do not ship with the standard library.

These packages use names under `Trifle\`. Their Git tags define their
versions. Install them through Whim's Git package manager.

For example, this command adds the command-line argument parser:

```console
whim add git+ssh://git@codeberg.org/trifle/args
```

The group includes:

- [`Trifle\Clock`](https://codeberg.org/trifle/clock) for clocks and sleep that
  tests can control
- [`Trifle\Diff`](https://codeberg.org/trifle/diff) for value and text diffs
- [`Trifle\SemVer`](https://codeberg.org/trifle/semver) for semantic versions
  and version requirements
- [`Trifle\Args`](https://codeberg.org/trifle/args) for command-line arguments

The [Trifle group](https://codeberg.org/trifle) lists all official packages.

After you add `Trifle\Diff` and load `vendor/autoload.whim`, you can compare two
sequences:

```whim,norun
use Trifle\Diff;
use Trifle\Diff\Operation;

$edits = Diff\diff::<string>(vec['a', 'b', 'c'], vec['a', 'c', 'd']);

foreach ($edits as $edit) {
  $marker = match ($edit->operation) {
    Operation::Keep => ' ',
    Operation::Delete => '-',
    Operation::Insert => '+',
  };

  write_line!($marker . ' ' . $edit->value);
}
```

See [Git Dependencies](dependencies.md) for manifests, locks, updates, and
autoloading.

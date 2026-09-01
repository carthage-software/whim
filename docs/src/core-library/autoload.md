# Autoloading

Whim asks one registered callback to load a missing symbol. The standard
library turns that callback into an ordered list of `Autoloader` values.

An autoloader receives the requested `SymbolKind` and full name. It returns
`true` when it handled the request and `false` when the next loader should try.

## Direct symbol maps

`Autoloader::withSymbolFile()` maps one full symbol name to a file.
`withSymbolFiles()` adds a dict of mappings. Both return a cloned loader and
leave the old value unchanged.

```whim,norun
use Whim\Autoload;
use Whim\Autoload\Autoloader;

$loader = new Autoloader()
  ->withSymbolFile('App\\User', directory!() . '/src/User.whim')
  ->withSymbolFile('App\\load_user', directory!() . '/src/functions.whim');

Autoload\register($loader);
```

When Whim requests a mapped name, the loader calls `require_once!` for its
file. The file must declare the requested symbol.

## A fallback loader

`withFallback()` sets a callable for names outside the direct map.

```whim,norun
use Whim\Autoload;
use Whim\Autoload\Autoloader;
use Whim\Symbol\SymbolKind;

$loader = new Autoloader()->withFallback(
  fn(SymbolKind $kind, string $name): bool {
    if ($name != 'App\\User') {
      return false;
    }

    require_once!(directory!() . '/src/User.whim');
    return true;
  },
);

Autoload\register($loader);
```

The kind tells the loader whether Whim needs a class, interface, enum, function,
constant, type alias, or newtype. One name cannot belong to two symbol kinds.

## The loader list

`Autoload\register()` appends a loader. `unregister()` removes every equal
entry and does nothing when no entry matches. `get_autoloaders()` returns the
current list in call order.

`Autoload\load_symbol($kind, $name)` asks the same chain used by the engine and
returns whether the symbol now exists.

A loader that returns `true` must define the requested symbol. If it does not,
Whim throws `UndefinedSymbolError`. Syntax, type, link, and top-level errors
from a loaded file pass to the caller.

## Generated dependency loader

`whim install` writes `vendor/autoload.whim`. Requiring it registers one loader
for the root project and installed packages:

```whim,norun
require_once!(directory!() . '/vendor/autoload.whim');
```

The generated loader contains fixed namespace maps. It does no network access,
manifest parsing, package resolution, or directory scan. `whim run` does not
look for it; the application must require it.

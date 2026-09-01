# Namespaces and Imports

A namespace prefixes each declaration in its body.

## File namespace

The common form applies from its declaration to the end of the file:

```whim
namespace App\Model;

final class User {}
```

The class name is `App\Model\User`.

## Braced namespace

A braced namespace limits the prefix to one block:

```whim
namespace App\Math {
  function double(int $value): int {
    return $value * 2;
  }
}

assert!(App\Math\double(21) == 42);
```

Declarations remain top-level even when a namespace block holds them.

## Imports

`use` imports one symbol under its last name:

```whim
namespace App;

use Whim\DateTime\Date;
use Whim\Json;

$date = Date::from('2026-08-21');
$json = Json\encode(dict['date' => $date->toString()]);
assert!($json != '');
```

An alias chooses another local name:

```whim,norun
use App\Model\User as ModelUser;
```

One declaration may import several names:

```whim,norun
use Whim\HTTP\Message\Request, Whim\HTTP\Message\Response;
use Whim\HTTP\Message\{FieldMap, ProtocolVersion};
```

The braced form shares the prefix before `{`. Either form permits an alias on
each item and an optional trailing comma in a braced list.

Whim uses one import form for every symbol kind. It has no `use function` or
`use const` form.

Two imports cannot claim the same local name. A declaration also cannot reuse
an imported local name.

## Imports do not load code

`use` changes name lookup only. It does not read a file or run an autoloader.
Whim loads code through `require!`, `require_once!`, or a registered autoloader.
The next chapter covers each path.

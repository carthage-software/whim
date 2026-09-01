# Built-in Attributes

Whim supplies attributes for call contracts and error reports.

## MustUse

`#[MustUse]` requires the caller to consume a return value. It accepts an
optional note.

```whim
use Whim\Marker\MustUse;

#[MustUse('store or send the token')]
function make_token(): string {
  return 'token';
}

$token = make_token();
```

Calling `make_token();` as a statement raises `DiscardedResultError`. Call
`discard!(make_token())` when discarding the value is intentional.

Whim rejects `#[MustUse]` on a callable that returns `void` or `never`, since
such a call has no result to consume.

## SensitiveParameter

`#[SensitiveParameter]` hides one argument in stack traces.

```whim
use Whim\Marker\SensitiveParameter;

function sign(string $message, #[SensitiveParameter] string $secret): string {
  return $message . ':' . $secret;
}
```

The call still receives the real value. Only diagnostic output gets a
`SensitiveParameterValue` in its place. `SensitiveParameterValue::getValue()`
returns the hidden value to code that already holds that wrapper.

## Deprecated

`#[Deprecated]` marks a symbol or member as old. Its first argument is the
version that marked it old. Its optional second argument explains what to use.

```whim
use Whim\Marker\Deprecated;

#[Deprecated('0.2.0', 'use encode()')]
function old_encode(string $value): string {
  return $value;
}
```

Using the symbol reports the version and note.

## TrackCaller

`#[TrackCaller]` moves an explicit throw site through marked wrappers to the
first outer call without the marker. This keeps a small public wrapper from
hiding the user's call site.

Use it on an API that checks arguments or forwards an error. Do not add it to a
callable that cannot throw and does not call user code.

## TraceBoundary

`#[TraceBoundary]` hides the marked frame and deeper implementation frames from
normal stack traces. `WHIM_FULL_TRACE=true whim app.whim` shows them.

Use it at a library boundary where lower frames add no useful action for the
caller. It does not catch, change, or suppress an error.

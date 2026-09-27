use std::path::Path;

use whim_runtime::engine::Engine;
use whim_runtime::engine::EngineConfiguration;

fn run(source: &str) {
    for optimize in [false, true] {
        let mut engine = Engine::new(EngineConfiguration {
            optimize,
            ..EngineConfiguration::default()
        });
        let result = engine.run_source(source, Path::new("/property-consumers.whim"));
        assert_eq!(result.exit_code(), 0, "optimization {optimize}: {result:?}");
    }
}

#[test]
fn property_reads_keep_values_and_copy_on_write_aliases() {
    run(r"
use Whim\Marker\NeverInline;
final class Store {
    public vec<(int, uint)> $items = vec[(3, 5u), (7, 11u)];
    public vec<vec<string>> $rows = vec[vec['first', 'second']];
    public dict<string, int> $counts = dict['first' => 3, 'second' => 7];
    #[NeverInline]
    public function count(): uint { return length!($this->items); }
    #[NeverInline]
    public function at(uint $index): (int, uint) { return $this->items[$index]; }
    #[NeverInline]
    public function row(uint $index): vec<string> { return $this->rows[$index]; }
    #[NeverInline]
    public function keys(): uint { return length!($this->counts); }
    #[NeverInline]
    public function append((int, uint) $value): uint {
        $count = length!($this->items);
        $this->items[] = $value;
        return $count;
    }
    #[NeverInline]
    public function take_first(): (int, uint) {
        $first = $this->items[0u];
        discard!(remove_last!($this->items));
        return $first;
    }
    #[NeverInline]
    public function update(uint $index, int $value): (int, uint) {
        $previous = $this->items[$index];
        $this->items[$index] = ($value, $index);
        return $previous;
    }
}
$store = new Store();
$snapshot = $store->items;
$first = $store->at(0u);
$row = $store->row(0u);
$sum = 0;
for ($index = 0; $index < 32; $index++) {
    assert!($store->count() == 2u);
    assert!($store->keys() == 2u);
    $old = $store->update(0u, $index);
    $sum += $old[0];
    assert!($store->at(0u) == ($index, 0u));
}
assert!($sum == 468);
assert!($snapshot == vec[(3, 5u), (7, 11u)]);
assert!($first == (3, 5u));
$store->rows[0u] = vec['changed'];
assert!($row == vec['first', 'second']);
assert!($store->row(0u) == vec['changed']);
assert!($store->append((40, 41u)) == 2u);
assert!($store->append((50, 51u)) == 3u);
assert!($store->take_first() == (31, 0u));
assert!($store->count() == 3u);
");
}

#[test]
fn collection_reads_preserve_object_lifetimes_and_unknown_values() {
    run(r"
use Whim\Marker\NeverInline;
final class Token {
    public static int $drops = 0;
    public function __construct(public string $name) {}
    public function __destruct(): void { self::$drops++; }
}
final class Objects {
    public vec<Token> $items = vec[];
    public vec $unknown = vec[];
    #[NeverInline]
    public function count(): uint { return length!($this->items); }
    #[NeverInline]
    public function at(uint $index): Token { return $this->items[$index]; }
    #[NeverInline]
    public function untyped(uint $index): mixed { return $this->unknown[$index]; }
    #[NeverInline]
    public function clear(): void {
        $saved = $this->items;
        assert!(length!($this->items) == 2u);
        $this->items = vec[];
        assert!(Token::$drops == 0);
        assert!($saved[1u]->name == 'second');
    }
}
#[NeverInline]
function exercise(): void {
    $objects = new Objects();
    $objects->items = vec[new Token('first'), new Token('second')];
    $objects->unknown = vec[vec[1, 2], 'text'];
    $selected = $objects->at(0u);
    assert!($objects->count() == 2u);
    assert!($objects->untyped(0u) == vec[1, 2]);
    assert!($objects->untyped(1u) == 'text');
    $objects->clear();
    assert!($selected->name == 'first');
    assert!(Token::$drops == 1);
}
exercise();
assert!(Token::$drops == 2);
");
}

#[test]
fn reads_keep_bounds_uninitialized_errors_and_catch_state() {
    run(r"
use Whim\Marker\NeverInline;
use Whim\Unwind\OutOfBoundsError;
use Whim\Unwind\TypeError;
use Whim\Unwind\UninitializedPropertyError;
final class Store {
    public vec<int> $items = vec[17];
    #[NeverInline]
    public function at(int $index): int { return $this->items[$index]; }
    #[NeverInline]
    public function first(): int { return $this->items[0u]; }
    #[NeverInline]
    public function guarded(int $index): mixed {
        $result = 'saved';
        try { $result = $this->items[$index]; }
        catch (OutOfBoundsError $_) { assert!($result == 'saved'); return null; }
        return $result;
    }
    #[NeverInline]
    public function guarded_first(): mixed {
        $result = 'saved';
        try { $result = $this->items[0u]; }
        catch (OutOfBoundsError $_) { assert!($result == 'saved'); return null; }
        return $result;
    }
}
final class Late {
    public vec<int> $items;
    #[NeverInline]
    public function count(): uint { return length!($this->items); }
    #[NeverInline]
    public function at(uint $index): int { return $this->items[$index]; }
}
final class Unknown {
    public mixed $items = false;
    #[NeverInline]
    public function count(): uint {
        $size = 9u;
        try { $size = length!($this->items); }
        catch (TypeError $error) { assert!($size == 9u); throw $error; }
        return $size;
    }
}
$store = new Store();
foreach (vec[-1, 1, 9223372036854775807] as $index) {
    $result = 'saved';
    $caught = false;
    try { $result = $store->at($index); }
    catch (OutOfBoundsError $_) { $caught = true; }
    assert!($caught && $result == 'saved');
    assert!($store->guarded($index) == null);
}
assert!($store->guarded(0) == 17);
assert!($store->first() == 17);
$store->items = vec[];
$caught = false;
try { $store->first(); } catch (OutOfBoundsError $_) { $caught = true; }
assert!($caught && $store->guarded_first() == null);
$late = new Late();
$caught = 0;
try { $late->count(); } catch (UninitializedPropertyError $_) { $caught++; }
try { $late->at(0u); } catch (UninitializedPropertyError $_) { $caught++; }
assert!($caught == 2);
$late->items = vec[23];
assert!($late->count() == 1u && $late->at(0u) == 23);
$unknown = new Unknown();
$caught = false;
try { $unknown->count(); } catch (TypeError $_) { $caught = true; }
assert!($caught);
$unknown->items = dict['ready' => 1];
assert!($unknown->count() == 1u);
");
}

#[test]
fn first_local_lengths_keep_reused_frame_lifetimes() {
    run(r"
use Whim\Marker\NeverInline;
use Whim\Reference\Weak;
use Whim\Unwind\Exception;
final class Token {
    public static int $drops = 0;
    public function __construct(public int $value) {}
    public function __destruct(): void { self::$drops++; }
}
final class Store {
    public vec<int> $items = vec[3, 7];
    #[NeverInline]
    public function count(): uint {
        $count = length!($this->items);
        return $count;
    }
    #[NeverInline]
    public function append(int $value): uint {
        $count = length!($this->items);
        $this->items[] = $value;
        return $count;
    }
}
#[NeverInline]
function count(Store $store): uint {
    $count = length!($store->items);
    return $count;
}
#[NeverInline]
function release(): Weak<Token> {
    $object = new Token(19);
    $callback = fn(): int => $object->value;
    $weak = new Weak::<Token>($object);
    assert!($callback() == 19);
    return $weak;
}
#[NeverInline]
function hold(Store $store): Weak<Token> {
    $object = new Token(23);
    $callback = fn(): int => $object->value;
    $weak = new Weak::<Token>($object);
    $drops = Token::$drops;
    for ($index = 0; $index < 4; $index++) {
        assert!($store->count() == 2u && count($store) == 2u);
        assert!($weak->get() != null && $callback() == 23);
        assert!(Token::$drops == $drops);
    }
    return $weak;
}
#[NeverInline]
function unwind(Store $store): void {
    $object = new Token(29);
    $callback = fn(): int => $object->value;
    assert!($callback() == 29 && $store->count() == 2u);
    throw new Exception('release this frame');
}
$store = new Store();
$snapshot = $store->items;
for ($round = 0; $round < 16; $round++) {
    $before = Token::$drops;
    $weak = release();
    assert!($weak->get() == null && Token::$drops == $before + 1);
    assert!($store->count() == 2u && count($store) == 2u);
    assert!($store->append($round) == 2u);
    assert!(remove_last!($store->items) == $round);
    $weak = hold($store);
    assert!($weak->get() == null && Token::$drops == $before + 2);
    try { unwind($store); } catch (Exception $_) {}
    assert!(Token::$drops == $before + 3);
    assert!($store->count() == 2u && count($store) == 2u);
    assert!($store->items == $snapshot);
}
assert!(Token::$drops == 48);
");
}

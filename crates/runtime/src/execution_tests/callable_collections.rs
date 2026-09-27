use super::run_both_modes;

const FIXTURE: &str = include_str!("../../../../tests/_fixtures/callable-collections.whim");

#[test]
fn foreach_callbacks_preserve_bound_receivers_and_captures() {
    let mut source = String::from(FIXTURE);
    source.push_str(
        r"
$receiver = new Receiver();
$bound = $receiver->accept(...);
$captured = fn(Event $event): void { $receiver->total += $event->value * 2; };
$event = new Event(7);
dispatch_vector(vec[$bound, $captured], $event);
assert!($receiver->total == 21);
dispatch_dictionary(dict['bound' => $bound, 'captured' => $captured], $event);
assert!($receiver->total == 42);
dispatch_tuple(($bound, $captured), $event);
assert!($receiver->total == 63);
$transform = $receiver->transform(...);
$offset = 5;
$add = fn(int $value): int => $value + $offset;
assert!(sum_vector(vec[$transform, $add], 4) == 76);
assert!(sum_dictionary(dict['bound' => $transform, 'captured' => $add], 4) == 76);
assert!(sum_tuple(($transform, $add), 4) == 76);
assert!(sum_vector(vec[], 4) == 0);
dispatch_vector(vec[], $event);
unknown_argument(vec[$bound], $event);
unknown_callback(vec[$bound], $event);
assert!($receiver->total == 77);
",
    );
    run_both_modes(&source, "/callable-collection-values.whim");
}

#[test]
fn foreach_callbacks_preserve_argument_return_and_must_use_errors() {
    let mut source = String::from(FIXTURE);
    source.push_str(
        r"
#[Whim\Marker\NeverInline]
function invalid_return(int $value): int { return 'wrong'; }
#[Whim\Marker\MustUse]
function required_result(int $value): int { return $value; }
$receiver = new Receiver();
$bound = $receiver->accept(...);
$event = new Event(7);
$caught = false;
try { unknown_argument(vec[$bound], 42); }
catch (Whim\Unwind\TypeError $error) { $caught = true; }
assert!($caught);
$caught = false;
try { unknown_callback(vec[fn(int $value): void {}], $event); }
catch (Whim\Unwind\TypeError $error) { $caught = true; }
assert!($caught);
$caught = false;
try { changed_callback(vec[$bound], fn(int $value): void {}, $event); }
catch (Whim\Unwind\TypeError $error) { $caught = true; }
assert!($caught);
assert!($receiver->total == 0);
$caught = false;
try { discard!(sum_vector(vec[invalid_return(...)], 1)); }
catch (Whim\Unwind\TypeError $error) { $caught = true; }
assert!($caught);
$caught = false;
try { discard_value(vec[required_result(...)], 1); }
catch (Whim\Unwind\DiscardedResultError $error) { $caught = true; }
assert!($caught);
",
    );
    run_both_modes(&source, "/callable-collection-errors.whim");
}

#[test]
fn foreach_output_assignments_keep_keys_values_and_call_targets() {
    let mut source = String::from(FIXTURE);
    source.push_str(
        r"
function text_result(int $value): string { return 'wrong'; }
assert!(dictionary_key(dict['first' => 42]) == 'first');
assert!(vector_key(vec[42]) == 0u);
assert!(tuple_key((42, 43)) == 0u);
assert!(generic_key(dict['first' => 42]) == 'first');
assert!(generic_key(vec[42]) == 0u);
assert!(generic_key((42, 43)) == 0u);
assert!(dictionary_value(dict['first' => 42]) == 42);
assert!(!refined_key(dict[1 => -1]));
assert!(refined_key(dict[1 => 2]));
$caught = false;
try { discard!(callback_from_key(dict['text_result' => fn(int $value): int => $value], 3)); }
catch (Whim\Unwind\TypeError $error) { $caught = true; }
assert!($caught);
",
    );
    run_both_modes(&source, "/foreach-output-assignments.whim");
}

#[test]
fn identity_partials_preserve_method_scope_types_and_argument_mapping() {
    run_both_modes(
        r"
use Whim\Marker\NeverInline;
class Scoped {
    protected static function name(): string { return 'base'; }
    public static function label(int $value): string { return static::name() . $value; }
}
final class Child extends Scoped {
    protected static function name(): string { return 'child'; }
}
final class Box<T> {
    public function __construct(private T $value) {}
    public function read(T $fallback): T { return $this->value; }
    public function choose<U>(U $value): U { return $value; }
    public function format(string $left, string $right = '!'): string {
        return $left . $this->value . $right;
    }
    public function closure(): fn(int): int {
        return fn(int $value): int => $value + ($this->value as int);
    }
}
#[NeverInline]
function invoke(fn $callback, mixed $value): mixed { return $callback($value); }
#[NeverInline]
function typed(fn(int): int $callback, int $value): int { return $callback($value); }
#[NeverInline]
function pair(fn $callback, mixed $left, mixed $right): mixed { return $callback($left, $right); }
$box = new Box::<int>(7);
$read = $box->read(...);
assert!(typed($read, 1) == 7);
assert!(invoke($read, 2) == 7);
$choose = $box->choose::<string>(...);
assert!(invoke($choose, 'chosen') == 'chosen');
assert!(invoke(Child::label(...), 3) == 'child3');
$captured = $box->closure();
$partial = $captured(?);
assert!(typed($partial, 5) == 12);
$format = $box->format(...);
assert!(invoke($format, '[') == '[7!');
assert!(pair($format, '[', ']') == '[7]');
$default = $box->format(?);
assert!(invoke($default, '(') == '(7!');
$reordered = $box->format(right: ?, left: ?);
assert!(pair($reordered, ')', '(') == '(7)');
$filled = $box->format(?, ']');
assert!(invoke($filled, '[') == '[7]');
$nested = $format(?, '}');
assert!(invoke($nested, '{') == '{7}');
foreach (vec[$read, $choose] as $callback) {
    $caught = false;
    try { discard!(invoke($callback, false)); }
    catch (Whim\Unwind\TypeError $error) { $caught = true; }
    assert!($caught);
}
$caught = false;
try { discard!($read()); }
catch (Whim\Unwind\ArgumentCountError $error) { $caught = true; }
assert!($caught);
$caught = false;
try { discard!(pair($read, 1, 2)); }
catch (Whim\Unwind\ArgumentCountError $error) { $caught = true; }
assert!($caught);
",
        "/identity-partial-methods.whim",
    );
}

#[test]
fn identity_partials_preserve_unwind_discard_and_receiver_lifetime() {
    run_both_modes(
        r"
use Whim\Marker\NeverInline;
final class Trace { public vec<string> $events = vec[]; }
final class Receiver {
    public function __construct(private Trace $trace) {}
    public function __destruct(): void { $this->trace->events[] = 'drop'; }
    public function read(int $value): int {
        $this->trace->events[] = 'read';
        return $value + 1;
    }
    public function fail(int $value): int {
        $this->trace->events[] = 'throw';
        throw new Whim\Unwind\Exception('failed');
    }
    #[Whim\Marker\MustUse]
    public function required(int $value): int {
        $this->trace->events[] = 'required';
        return $value;
    }
    public function invalid(int $value): int { return 'wrong'; }
}
#[NeverInline]
function overwrite(Trace $trace): int {
    $callback = new Receiver($trace)->read(...);
    $callback = $callback(4);
    return $callback;
}
#[NeverInline]
function invoke(fn(int): int $callback): int { return $callback(1); }
#[NeverInline]
function discard(fn(int): int $callback): void { $callback(1); }
#[NeverInline]
function catch_throw(Trace $trace): bool {
    try { discard!(invoke(new Receiver($trace)->fail(...))); }
    catch (Whim\Unwind\Exception $error) { return true; }
    return false;
}
#[NeverInline]
function catch_discard(Trace $trace): bool {
    try { discard(new Receiver($trace)->required(...)); }
    catch (Whim\Unwind\DiscardedResultError $error) { return true; }
    return false;
}
#[NeverInline]
function catch_invalid_return(Trace $trace): bool {
    try { discard!(invoke(new Receiver($trace)->invalid(...))); }
    catch (Whim\Unwind\TypeError $error) { return true; }
    return false;
}
$trace = new Trace();
assert!(overwrite($trace) == 5);
assert!($trace->events == vec['read', 'drop']);
$trace->events = vec[];
assert!(catch_throw($trace));
assert!($trace->events == vec['throw', 'drop']);
$trace->events = vec[];
assert!(catch_discard($trace));
assert!($trace->events == vec['required', 'drop']);
$trace->events = vec[];
assert!(catch_invalid_return($trace));
assert!($trace->events == vec['drop']);
",
        "/identity-partial-lifetimes.whim",
    );
}

#[test]
fn bound_method_frames_keep_inherited_and_bound_type_environments() {
    run_both_modes(
        r"
use Whim\Marker\NeverInline;
class Parent<T> {
    public static int $drops = 0;
    private string $prefix = 'private-';
    public function __construct(private T $stored) {}
    public function __destruct(): void { Parent::$drops++; }
    protected static function label(): string { return 'parent'; }
    public function combine<U>(T $left, U $right): (T, U, T, string) {
        return ($left, $right, $this->stored, $this->prefix . static::label());
    }
    public function defaulted<U = int>(U $value): U { return $value; }
    public function invalid<U>(U $value): U { return $this->stored; }
    #[Whim\Marker\Frameless]
    public function literal(): int { return 42; }
}
final class Child<X> extends Parent<int> {
    protected static function label(): string { return 'child'; }
}
#[NeverInline]
function pair(fn(int, string): (int, string, int, string) $callback): (int, string, int, string) {
    return $callback(3, 'right');
}
#[NeverInline]
function integer(fn(int): int $callback): int { return $callback(5); }
#[NeverInline]
function text(fn(string): string $callback): string { return $callback('fallback'); }
#[NeverInline]
function zero(fn(): int $callback): int { return $callback(); }
#[NeverInline]
function temporary_receiver(): int { return zero(new Child::<bool>(9)->literal(...)); }
$value = new Child::<bool>(7);
$combine = $value->combine::<string>(?, ?);
assert!(pair($combine) == (3, 'right', 7, 'private-child'));
assert!(pair($value->combine::<string>(...)) == (3, 'right', 7, 'private-child'));
assert!(integer($value->defaulted(...)) == 5);
$caught = false;
try { discard!(text($value->invalid::<string>(?))); }
catch (Whim\Unwind\TypeError $_) { $caught = true; }
assert!($caught);
assert!(temporary_receiver() == 42);
assert!(Parent::$drops == 1);
",
        "/bound-method-environments.whim",
    );
}

#[test]
fn bound_method_frames_run_where_checks_before_defaults_and_body() {
    run_both_modes(
        r"
use Whim\Marker\NeverInline;
final class Effects {
    public static int $defaults = 0;
    public static int $bodies = 0;
}
#[NeverInline]
function suffix(): string { Effects::$defaults++; return '!'; }
final class Formatter<T> {
    public function format(string $left, string $right = suffix()): string where T: int {
        Effects::$bodies++;
        return $left . $right;
    }
    public function invalid(string $left, int $right = 'wrong'): string {
        Effects::$bodies++;
        return $left . $right;
    }
}
#[NeverInline]
function one(fn(string): string $callback): string { return $callback('['); }
#[NeverInline]
function two(fn(string, string): string $callback): string { return $callback('[', ']'); }
$valid = new Formatter::<int>();
assert!(one($valid->format(?)) == '[!');
assert!(Effects::$defaults == 1);
assert!(Effects::$bodies == 1);
assert!(two($valid->format(?, ?)) == '[]');
assert!(Effects::$defaults == 1);
assert!(Effects::$bodies == 2);
$caught = false;
try { discard!(one($valid->invalid(?))); }
catch (Whim\Unwind\TypeError $_) { $caught = true; }
assert!($caught);
assert!(Effects::$bodies == 2);
$invalid = new Formatter::<string>();
$callback = $invalid->format(?);
$caught = false;
try { discard!(one($callback)); }
catch (Whim\Unwind\TypeError $_) { $caught = true; }
assert!($caught);
$caught = false;
try { discard!(two($invalid->format(?, ?))); }
catch (Whim\Unwind\TypeError $_) { $caught = true; }
assert!($caught);
assert!(Effects::$defaults == 1);
assert!(Effects::$bodies == 2);
",
        "/bound-method-defaults.whim",
    );
}

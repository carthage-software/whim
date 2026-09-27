use super::run_both_modes;

#[path = "../../../../tests/_fixtures/return_contracts.rs"]
mod fixtures;

#[test]
fn generic_returns_preserve_nullable_collection_and_invalid_values() {
    run_both_modes(
        r"
use Whim\Marker\NeverInline;
type Integers = vec<int>;
#[NeverInline]
function id<T>(T $value): T { return $value; }
#[NeverInline]
function coalesced<T>(T $value): int { return id::<T>($value) ?? 7; }
#[NeverInline]
function concrete(int $value): int { return id::<int>($value) ?? 0; }
#[NeverInline]
function first(Integers $value): int { return id::<Integers>($value)[0]; }
#[NeverInline]
function invalid<T>(mixed $value): T { return $value; }
#[NeverInline]
function invalid_caller(): int { return invalid::<int>('wrong') ?? 0; }
assert!(coalesced::<int>(42) == 42);
assert!(concrete(42) == 42);
assert!(coalesced::<null|int>(null) == 7);
assert!(coalesced::<null|int>(42) == 42);
assert!(first(vec[3, 4]) == 3);
$values = vec[1, 2];
$copy = id::<Integers>($values);
$copy[0] = 'changed';
assert!($values == vec[1, 2]);
assert!(!($copy is Integers));
$caught = false;
try { invalid_caller(); } catch (Whim\Unwind\TypeError $error) { $caught = true; }
assert!($caught);
$caught = false;
try { coalesced::<string>('wrong'); } catch (Whim\Unwind\TypeError $error) { $caught = true; }
assert!($caught);
",
        "/generic-returns.whim",
    );
}

#[test]
fn structural_returns_preserve_errors_and_copy_on_write() {
    let mut source = String::from(fixtures::RETURNS);
    source.push_str(
        r"
assert!(record() == dict['a' => 1, 'b' => 'hello', 'c' => (1, 2)]);
assert!(record_rest() == dict['a' => 1, 'extra' => 2]);
assert!(duplicate_key() == dict['a' => 1]);
assert!(bool_rest() == dict['a' => 1, true => 2, false => 3]);
assert!(distinct_keys() == dict[1 => 1, '1' => 2]);
assert!(vector_tail() == vec[1, 2, 3]);
assert!(tuple_tail() == (1, 2, 3));
assert!(tuple_array() == (1, 2));
assert!(cow_alias() == dict['a' => 1]);
assert!(nested_cow_alias() == vec[dict['a' => 1]]);
assert!(nested_child_alias() == vec[dict['a' => 1]]);
assert!(escaped_child() == vec[dict['a' => 1]]);
assert!(unknown_key('a') == dict['a' => 1]);
assert!(callback_result(fn(): dict => dict['a' => 1]) == vec[dict['a' => 1]]);
",
    );
    for name in fixtures::CHECKED {
        let call = match *name {
            "unknown_key" => "unknown_key('wrong')".to_owned(),
            "callback_result" => "callback_result(fn(): dict => dict['a' => 0])".to_owned(),
            name => format!("{name}()"),
        };
        source.push_str(&format!(
            "$caught = false; try {{ {call}; }} catch (Whim\\Unwind\\TypeError $error) {{ $caught = true; }} assert!($caught);\n"
        ));
    }
    run_both_modes(&source, "/return-proofs.whim");
}

#[test]
fn incoming_capture_moves_release_after_return_and_repeated_calls() {
    run_both_modes(
        r"
use Whim\Marker\NeverInline;
final class Token {
    public static int $drops = 0;
    public function __destruct(): void { self::$drops++; }
}
#[NeverInline]
function exercise(): void {
    $object = new Token();
    $build = fn(int $ignored): (int, Token, int) => (1, $object, 2);
    $before = Token::$drops;
    for ($index = 0; $index < 4; $index++) {
        $result = $build($index);
        assert!($result[0] == 1 && $result[1] is Token && $result[2] == 2);
        assert!(Token::$drops == $before);
    }
}
for ($round = 0; $round < 8; $round++) {
    exercise();
    assert!(Token::$drops == $round + 1);
}
",
        "/incoming-capture-return.whim",
    );
}

#[test]
fn incoming_capture_moves_release_after_unwind_and_reused_frames() {
    run_both_modes(
        r"
use Whim\Marker\NeverInline;
use Whim\Reference\Weak;
use Whim\Unwind\DivisionByZeroError;
final class Token {
    public static int $drops = 0;
    public static mixed $held = null;
    public function __destruct(): void { self::$drops++; }
}
#[NeverInline]
function exercise(): Weak<Token> {
    $object = new Token();
    $weak = new Weak::<Token>($object);
    $raise = fn(int $zero): float {
        Token::$held = (1, $object, 2);
        return $zero / $zero;
    };
    $before = Token::$drops;
    for ($index = 0; $index < 4; $index++) {
        $caught = false;
        try { discard!($raise(0)); }
        catch (DivisionByZeroError $_) { $caught = true; }
        assert!($caught && Token::$held[1] is Token);
        Token::$held = null;
        assert!(Token::$drops == $before && $weak->get() != null);
    }
    return $weak;
}
for ($round = 0; $round < 8; $round++) {
    $weak = exercise();
    assert!(Token::$drops == $round + 1 && $weak->get() == null);
}
",
        "/incoming-capture-unwind.whim",
    );
}

pub(super) const RETURNS: &str = r"
use Whim\Marker\NeverInline;

#[NeverInline]
function record(): dict['a' => 1.., 'b' => string&!'', 'c' => (1, 1..=10), ...<string, int>] {
    return dict['a' => 1, 'b' => 'hello', 'c' => (1, 2)];
}
#[NeverInline]
function record_rest(): dict['a' => 1, ...<string&!'', 1..=3>] {
    return dict['a' => 1, 'extra' => 2];
}
#[NeverInline]
function duplicate_key(): dict['a' => 1] {
    $key = 'a';
    return dict[$key => 'discarded', 'a' => 1];
}
#[NeverInline]
function bool_rest(): dict['a' => 1, ...<bool, 1..=3>] {
    return dict['a' => 1, true => 2, false => 3];
}
#[NeverInline]
function distinct_keys(): dict[1 => 1, '1' => 2] { return dict[1 => 1, '1' => 2]; }
#[NeverInline]
function vector_tail(): vec[1, ...1..=3] { return vec[1, 2, 3]; }
#[NeverInline]
function tuple_tail(): (1, ...1..=3) { return (1, 2, 3); }
#[NeverInline]
function tuple_array(): array<0..=1, 1..=2> { return (1, 2); }
#[NeverInline]
function cow_alias(): dict['a' => 1] {
    $value = dict['a' => 1];
    $copy = $value;
    $value['a'] = 0;
    return $copy;
}
#[NeverInline]
function nested_cow_alias(): vec[dict['a' => 1]] {
    $value = vec[dict['a' => 1]];
    $copy = $value;
    $value[0]['a'] = 0;
    return $copy;
}
#[NeverInline]
function nested_child_alias(): vec[dict['a' => 1]] {
    $child = dict['a' => 1];
    $value = vec[$child];
    $child['a'] = 0;
    return $value;
}
#[NeverInline]
function change_copy(dict $value): void { $value['a'] = 0; }
#[NeverInline]
function escaped_child(): vec[dict['a' => 1]] {
    $child = dict['a' => 1];
    $value = vec[$child];
    change_copy($child);
    return $value;
}

#[NeverInline]
function invalid_missing(): dict['a' => 1] { return dict[]; }
#[NeverInline]
function invalid_extra(): dict['a' => 1] { return dict['a' => 1, 'extra' => 2]; }
#[NeverInline]
function invalid_rest_key(): dict['a' => 1, ...<string, int>] { return dict['a' => 1, 2 => 2]; }
#[NeverInline]
function invalid_rest_value(): dict['a' => 1, ...<string, int>] { return dict['a' => 1, 'b' => 'bad']; }
#[NeverInline]
function invalid_duplicate(): dict['a' => 1] {
    $key = 'a';
    return dict[$key => 1, 'a' => 0];
}
#[NeverInline]
function invalid_range(): dict['a' => 1..] { return dict['a' => 0]; }
#[NeverInline]
function invalid_empty_string(): dict['a' => string&!''] { return dict['a' => '']; }
#[NeverInline]
function invalid_tuple_member(): dict['a' => (1, 1..=10)] { return dict['a' => (1, 11)]; }
#[NeverInline]
function invalid_tuple_arity(): dict['a' => (1, 2)] { return dict['a' => (1, 2, 3)]; }
#[NeverInline]
function invalid_vector_tail(): vec[1, ...1..=3] { return vec[1, 4]; }
#[NeverInline]
function invalid_vector_length(): vec[1, 2] { return vec[1]; }
#[NeverInline]
function invalid_tuple_tail(): (1, ...1..=3) { return (1, 4); }
#[NeverInline]
function invalid_tuple_array_key(): array<string, int> { return (1, 2); }
#[NeverInline]
function invalid_mutation(): dict['a' => 1] {
    $value = dict['a' => 1];
    $value['a'] = 0;
    return $value;
}
#[NeverInline]
function invalid_nested_mutation(): vec[dict['a' => 1]] {
    $value = vec[dict['a' => 1]];
    $value[0]['a'] = 0;
    return $value;
}
#[NeverInline]
function invalid_copy_mutation(): dict['a' => 1] {
    $value = dict['a' => 1];
    $copy = $value;
    $copy['a'] = 0;
    return $copy;
}
#[NeverInline]
function invalid_duplicate_shape(): dict['a' => 1, 'a' => 1] { return dict['a' => 1]; }
#[NeverInline]
function invalid_bool_key(): dict[1 => 1] { return dict[true => 1]; }
#[NeverInline]
function boolean_shape(): dict[true => 1, false => 2, 1 => 3, '1' => 4] {
    return dict[true => 1, false => 2, 1 => 3, '1' => 4];
}
#[NeverInline]
function invalid_boolean_shape(): dict[true => 1] { return dict[1 => 1]; }
newtype Key = string;
#[NeverInline]
function invalid_rest_newtype(): dict['a' => 1, ...<Key, int>] {
    return dict['a' => 1, 'b' => 2];
}
#[NeverInline]
function unknown_key(string $key): dict['a' => 1] { return dict[$key => 1]; }
#[NeverInline]
function callback_result(fn(): dict $callback): vec[dict['a' => 1]] {
    return vec[$callback()];
}
";

pub(super) const CHECKED: &[&str] = &[
    "invalid_missing",
    "invalid_extra",
    "invalid_rest_key",
    "invalid_rest_value",
    "invalid_duplicate",
    "invalid_range",
    "invalid_empty_string",
    "invalid_tuple_member",
    "invalid_tuple_arity",
    "invalid_vector_tail",
    "invalid_vector_length",
    "invalid_tuple_tail",
    "invalid_tuple_array_key",
    "invalid_mutation",
    "invalid_nested_mutation",
    "invalid_copy_mutation",
    "invalid_duplicate_shape",
    "invalid_bool_key",
    "invalid_boolean_shape",
    "invalid_rest_newtype",
    "unknown_key",
    "callback_result",
];

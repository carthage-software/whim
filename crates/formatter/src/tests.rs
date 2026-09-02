//! Formatter tests.

use std::thread::Builder;

use whim_syn::arena::LocalArena;
use whim_syn::cst::node::Node;
use whim_syn::cst::walker::deepest_path;
use whim_syn::parser::MAX_STRUCTURAL_DEPTH;
use whim_syn::parser::parse;

use crate::FormatSettings;
use crate::format as format_source;

fn format(source: &str) -> String {
    let arena = LocalArena::new();

    format_source(&arena, source, FormatSettings::default())
        .expect("source should parse")
        .to_string()
}

#[track_caller]
fn assert_formats(source: &str, expected: &str) {
    let once = format(source);
    assert_eq!(once, expected, "unexpected formatting");

    let twice = format(&once);
    assert_eq!(once, twice, "formatting is not idempotent");

    let arena = LocalArena::new();
    format_source(&arena, &once, FormatSettings::default())
        .expect("formatted output should re-parse");
}

/// Asserts formatting is idempotent and re-parses, without pinning the exact
/// output. Useful for large samples.
#[track_caller]
fn assert_idempotent(source: &str) {
    let once = format(source);
    let twice = format(&once);
    assert_eq!(once, twice, "formatting is not idempotent:\n{once}");

    let arena = LocalArena::new();
    format_source(&arena, &once, FormatSettings::default())
        .expect("formatted output should re-parse");
}

#[test]
fn empty_program_is_empty() {
    assert_eq!(format(""), "");
    assert_eq!(format("   \n\n  \n"), "");
}

#[test]
fn simple_assignment() {
    assert_formats("$x=1;", "$x = 1;\n");
}

#[test]
fn collapses_blank_lines() {
    assert_formats("$a = 1;\n\n\n\n$b = 2;\n", "$a = 1;\n\n$b = 2;\n");
}

#[test]
fn preserves_single_blank_line() {
    assert_formats("$a = 1;\n\n$b = 2;\n", "$a = 1;\n\n$b = 2;\n");
}

#[test]
fn binary_precedence_is_preserved() {
    assert_formats("$x = 1 + 2 * 3;\n", "$x = 1 + 2 * 3;\n");
    assert_formats("$x = (1 + 2) * 3;\n", "$x = (1 + 2) * 3;\n");
    assert_formats(
        "$x=1+2|>double(...)==6;",
        "$x = 1 + 2 |> double(...) == 6;\n",
    );
    assert_formats(
        "$x=1|>(fn(int $v):int=>$v+1);",
        "$x = 1 |> (fn(int $v): int => $v + 1);\n",
    );
    assert_formats(
        r#"class A { public function f(): bool { return ($this->bytes[0] == "\x0a" || ($this->bytes[0] == "\xac" && (Str\ord($this->bytes[1]) & 0xf0) == 0x10) || ($this->bytes[0] == "\xc0" && $this->bytes[1] == "\xa8")); } }"#,
        concat!(
            "class A {\n",
            "  public function f(): bool {\n",
            "    return (\n",
            "      $this->bytes[0] == \"\\x0a\"\n",
            "      || (\n",
            "        $this->bytes[0] == \"\\xac\"\n",
            "        && (Str\\ord($this->bytes[1]) & 0xf0) == 0x10\n",
            "      )\n",
            "      || ($this->bytes[0] == \"\\xc0\" && $this->bytes[1] == \"\\xa8\")\n",
            "    );\n",
            "  }\n",
            "}\n",
        ),
    );
}

#[test]
fn a_breaking_rhs_stays_with_its_binary_operator() {
    assert_formats(
        concat!(
            "assert!($iterated==vec[(0,('Content-Type','text/plain')),(1,",
            "('Set-Cookie','session=one')),(2,('set-cookie','theme=dark'))]);",
        ),
        concat!(
            "assert!(\n",
            "  $iterated == vec[\n",
            "    (0, ('Content-Type', 'text/plain')),\n",
            "    (1, ('Set-Cookie', 'session=one')),\n",
            "    (2, ('set-cookie', 'theme=dark')),\n",
            "  ],\n",
            ");\n",
        ),
    );
}

#[test]
fn double_negation_keeps_space() {
    assert_formats("$x = - -$y;\n", "$x = - -$y;\n");
}

#[test]
fn dictionary_pattern_keys_preserve_negative_integer_spelling() {
    assert_formats(
        "$x=match($v){dict[-0x10=>_,-0b1=>_,-1_000=>_]=>1,_=>0};",
        concat!(
            "$x = match ($v) {\n",
            "  dict[-0x10 => $_, -0b1 => $_, -1_000 => $_] => 1,\n",
            "  $_ => 0,\n",
            "};\n",
        ),
    );
}

#[test]
fn prefers_single_quotes() {
    assert_formats("$x = \"hello\";\n", "$x = 'hello';\n");
}

#[test]
fn preserves_quotes_with_escapes() {
    assert_idempotent("$x = \"a\\nb\";\n");
}

#[test]
fn function_with_body() {
    assert_formats(
        "function foo():string{return 'bar';}",
        "function foo(): string {\n  return 'bar';\n}\n",
    );
}

#[test]
fn if_else_chain() {
    let source = "if ($a) { foo(); } else if ($b) { bar(); } else { baz(); }";
    let expected = concat!(
        "if ($a) {\n",
        "  foo();\n",
        "} else if ($b) {\n",
        "  bar();\n",
        "} else {\n",
        "  baz();\n",
        "}\n",
    );
    assert_formats(source, expected);
}

#[test]
fn class_with_members() {
    let source = concat!(
        "final class Point{public function __construct(private int $x,private int $y){}",
        "public function sum():int{return $this->x+$this->y;}}",
    );
    let expected = concat!(
        "final class Point {\n",
        "  public function __construct(private int $x, private int $y) {}\n",
        "\n",
        "  public function sum(): int {\n",
        "    return $this->x + $this->y;\n",
        "  }\n",
        "}\n",
    );
    assert_formats(source, expected);

    let spaced_source = concat!(
        "final class Point {\n",
        "    public function a(): int { return 1; }\n",
        "\n",
        "    public function b(): int { return 2; }\n",
        "}\n",
    );
    assert_idempotent(spaced_source);
}

#[test]
fn separates_consecutive_methods() {
    assert_formats(
        "interface Reader { public function read(): string; public function close(): void; }",
        concat!(
            "interface Reader {\n",
            "  public function read(): string;\n",
            "\n",
            "  public function close(): void;\n",
            "}\n",
        ),
    );
}

#[test]
fn separates_a_multiline_statement_from_the_next_statement() {
    assert_formats(
        "function run(): void { if ($ready) { start(); } finish(); }",
        concat!(
            "function run(): void {\n",
            "  if ($ready) {\n",
            "    start();\n",
            "  }\n",
            "\n",
            "  finish();\n",
            "}\n",
        ),
    );
}

#[test]
fn a_multiline_closure_argument_does_not_add_an_invisible_indent() {
    assert_formats(
        concat!(
            "function run(): void {",
            "foreach ($values as $value) {",
            "Async\\spawn::<null>(function(): null { return null; })->ignore();",
            "}",
            "}",
        ),
        concat!(
            "function run(): void {\n",
            "  foreach ($values as $value) {\n",
            "    Async\\spawn::<null>(function(): null {\n",
            "      return null;\n",
            "    })->ignore();\n",
            "  }\n",
            "}\n",
        ),
    );
}

#[test]
fn short_vec_stays_inline() {
    assert_formats("$x = vec[1, 2, 3];\n", "$x = vec[1, 2, 3];\n");
}

#[test]
fn long_vec_breaks_with_trailing_comma() {
    let source = concat!(
        "$x = vec['alpha', 'bravo', 'charlie', 'delta', 'echo', 'foxtrot', 'golf', 'hotel', ",
        "'india', 'juliet', 'kilo', 'lima'];\n",
    );
    let expected = concat!(
        "$x = vec[\n",
        "  'alpha',\n",
        "  'bravo',\n",
        "  'charlie',\n",
        "  'delta',\n",
        "  'echo',\n",
        "  'foxtrot',\n",
        "  'golf',\n",
        "  'hotel',\n",
        "  'india',\n",
        "  'juliet',\n",
        "  'kilo',\n",
        "  'lima',\n",
        "];\n",
    );
    assert_formats(source, expected);
}

#[test]
fn match_expression() {
    let source = "$x = match ($code) { 200 => 'ok', 404 => 'missing', _ => 'error' };";
    let expected = concat!(
        "$x = match ($code) {\n",
        "  200 => 'ok',\n",
        "  404 => 'missing',\n",
        "  $_ => 'error',\n",
        "};\n",
    );
    assert_formats(source, expected);
    assert_formats(
        "$x=match($value){$circle @ Circle=>$circle->radius,($number @ int,$_ @ 5)=>$number,vec[$first @ int,...string]=>$first,$_=>0};",
        concat!(
            "$x = match ($value) {\n",
            "  $circle @ Circle => $circle->radius,\n",
            "  ($number @ int, $_ @ 5) => $number,\n",
            "  vec[$first @ int, ...string] => $first,\n",
            "  $_ => 0,\n",
            "};\n",
        ),
    );
    assert_formats(
        "$x=match($this){self::Low=>'low',self::High=>'high'};",
        concat!(
            "$x = match ($this) {\n",
            "  self::Low => 'low',\n",
            "  self::High => 'high',\n",
            "};\n",
        ),
    );
    assert_formats(
        "$x=match($value){true=>break 2,false=>continue};",
        concat!(
            "$x = match ($value) {\n",
            "  true => break 2,\n",
            "  false => continue,\n",
            "};\n",
        ),
    );
    assert_formats(
        "$x=match($value){true=>return 42,false=>return};",
        concat!(
            "$x = match ($value) {\n",
            "  true => return 42,\n",
            "  false => return,\n",
            "};\n",
        ),
    );
    assert_formats(
        "$x=match($value){Circle @ $circle=>$circle->radius,vec<int> @ ($first,...$rest)=>$first,_=>0};",
        concat!(
            "$x = match ($value) {\n",
            "  $circle @ Circle => $circle->radius,\n",
            "  ($first, ...$rest) @ vec<int> => $first,\n",
            "  $_ => 0,\n",
            "};\n",
        ),
    );
    assert_formats(
        "$x=match((dict['right'=>42],)){(dict['left'=>$found @ int]|dict['right'=>$found @ int],)=>$found,$_=>0};",
        concat!(
            "$x = match ((dict['right' => 42],)) {\n",
            "  (dict['left' => $found @ int] | dict['right' => $found @ int],) => $found,\n",
            "  $_ => 0,\n",
            "};\n",
        ),
    );
}

#[test]
fn closure_and_short_closure() {
    assert_formats("$f = fn($x)=>$x*2;\n", "$f = fn($x) => $x * 2;\n");
    assert_formats(
        "$f=fn(int $x):int{$result=$x*2;return $result;};",
        concat!(
            "$f = fn(int $x): int {\n",
            "  $result = $x * 2;\n",
            "  return $result;\n",
            "};\n",
        ),
    );
    assert_formats(
        "$g = function ($x) use ($y): int { return $x + $y; };",
        "$g = function($x) use ($y): int {\n  return $x + $y;\n};\n",
    );
    assert_formats(
        "$f = function<T: Countable + Traversable> (): void {};",
        "$f = function<T: Countable + Traversable>(): void {};\n",
    );
    assert_formats(
        "$f=function():dict<string|int|bool,mixed>{return dict[];};",
        concat!(
            "$f = function(): dict<string|int|bool, mixed> {\n",
            "  return dict[];\n",
            "};\n",
        ),
    );
    assert_formats(
        "$f = function (/* no parameters */): void {};",
        "$f = function(/* no parameters */): void {};\n",
    );
}

#[test]
fn leading_comment_is_kept() {
    let source = "// a comment\n$x = 1;\n";
    assert_formats(source, "// a comment\n$x = 1;\n");
}

#[test]
fn trailing_comment_is_kept() {
    let source = "$x = 1; // set x\n";
    assert_formats(source, "$x = 1; // set x\n");
}

#[test]
fn comment_at_end_of_file_is_kept() {
    assert_formats("$x = 1;\n// trailing\n", "$x = 1;\n// trailing\n");
}

#[test]
fn comment_between_statements_is_kept() {
    assert_formats("$a = 1;\n// note\n$b = 2;\n", "$a = 1;\n// note\n$b = 2;\n");
}

#[test]
fn dangling_comment_in_empty_block_is_kept() {
    assert_formats(
        "function f(): void { /* nothing yet */ }",
        "function f(): void {\n  /* nothing yet */\n}\n",
    );
}

#[test]
fn docblock_is_reindented() {
    let source = "/**\n * Doc.\n */\nfunction foo(): void {}\n";
    let expected = "/**\n * Doc.\n */\nfunction foo(): void {}\n";
    assert_formats(source, expected);
}

#[test]
fn shebang_is_preserved() {
    let source = "#!/usr/bin/env whim\n$x = 1;\n";
    assert_formats(source, "#!/usr/bin/env whim\n$x = 1;\n");
}

#[test]
fn enum_with_cases() {
    let source = "enum Suit: string { case Hearts = 'H'; case Spades = 'S'; }";
    let expected = concat!(
        "enum Suit: string {\n",
        "  case Hearts = 'H';\n",
        "  case Spades = 'S';\n",
        "}\n",
    );
    assert_formats(source, expected);
}

#[test]
fn namespace_and_use() {
    let source = "namespace App\\Service; use App\\Mailer; use App\\{Logger, Cache};";
    let expected = concat!(
        "namespace App\\Service;\n",
        "\n",
        "use App\\Mailer;\n",
        "use App\\{Logger, Cache};\n",
    );
    assert_formats(source, expected);
}

#[test]
fn foreach_and_for_and_while() {
    assert_idempotent("foreach ($items as $key => $value) { print($value); }");
    assert_idempotent("for ($i = 0; $i < 10; $i++) { print($i); }");
    assert_idempotent("while ($i < 10) { $i += 1; }");
    assert_idempotent("do { $i += 1; } while ($i < 10);");
    assert_formats("for (;;) { go(); }", "for (; ;) {\n  go();\n}\n");
}

#[test]
fn redundant_header_parentheses_are_removed() {
    assert_formats("if ($x > 0) { foo(); }", "if ($x > 0) {\n  foo();\n}\n");
    assert_formats("while ($ok) { work(); }", "while ($ok) {\n  work();\n}\n");
    assert_formats(
        "do { step(); } while ($ok);",
        "do {\n  step();\n} while ($ok);\n",
    );
    assert_formats(
        "foreach (($items) as $x) { each($x); }",
        "foreach ($items as $x) {\n  each($x);\n}\n",
    );
}

#[test]
fn try_catch_finally() {
    assert_idempotent("try { risky(); } catch (NotFound $e) { log($e); } finally { cleanup(); }");
    assert_idempotent(
        "try { risky(); } catch (NotFound $e) { recover($e); } else { commit(); } finally { cleanup(); }",
    );
    assert_formats(
        "try{risky();}catch(RuntimeException $e)if($e->getCode()==404){recover();}",
        "try {\n  risky();\n} catch (RuntimeException $e) if ($e->getCode() == 404) {\n  recover();\n}\n",
    );
    assert_formats(
        "try{risky();}else{commit();}",
        "try {\n  risky();\n} else {\n  commit();\n}\n",
    );
    assert_formats(
        concat!(
            "try {} catch (E0 $e) {} catch (E1 $e) {} catch (E2 $e) {} ",
            "catch (E3 $e) {} catch (E4 $e) {} catch (E5 $e) {} catch (E6 $e) {} ",
            "catch (E7 $e) {} catch (E8 $e) {} catch (E9 $e) {} catch (E10 $e) {} ",
            "catch (E11 $e) {} catch (E12 $e) {}",
        ),
        concat!(
            "try {\n",
            "} catch (E0 $e) {\n",
            "} catch (E1 $e) {\n",
            "} catch (E2 $e) {\n",
            "} catch (E3 $e) {\n",
            "} catch (E4 $e) {\n",
            "} catch (E5 $e) {\n",
            "} catch (E6 $e) {\n",
            "} catch (E7 $e) {\n",
            "} catch (E8 $e) {\n",
            "} catch (E9 $e) {\n",
            "} catch (E10 $e) {\n",
            "} catch (E11 $e) {\n",
            "} catch (E12 $e) {\n",
            "}\n",
        ),
    );
}

#[test]
fn using_and_drop() {
    assert_formats(
        "using($file=open(),($input,$output)=channels(),){drop!($file,$input,);}",
        "using ($file = open(), ($input, $output) = channels()) {\n  drop!($file, $input);\n}\n",
    );
}

#[test]
fn redundant_catch_type_parentheses_are_removed() {
    assert_formats(
        "try { a(); } catch ((Foo) $e) { b(); }",
        "try {\n  a();\n} catch (Foo $e) {\n  b();\n}\n",
    );
    assert_formats(
        "try { a(); } catch ((Foo)) { b(); }",
        "try {\n  a();\n} catch (Foo) {\n  b();\n}\n",
    );
    assert_formats(
        "try { a(); } catch ((Foo|Bar) $e) { b(); }",
        "try {\n  a();\n} catch (Foo|Bar $e) {\n  b();\n}\n",
    );
}

#[test]
fn type_operations() {
    assert_idempotent("$b = $value is int;");
    assert_idempotent("$c = $input as string;");
    assert_idempotent("$d = $raw ?as float ?? 0.0;");
    assert_formats(
        "if ($name is 'connection'|'keep-alive'|'proxy-connection'|'transfer-encoding'|'upgrade') {}",
        concat!(
            "if (\n",
            "  $name is 'connection'\n",
            "    | 'keep-alive'\n",
            "    | 'proxy-connection'\n",
            "    | 'transfer-encoding'\n",
            "    | 'upgrade'\n",
            ") {}\n",
        ),
    );
}

#[test]
fn shape_types_keep_their_rest_elements() {
    assert_formats(
        "function a(vec[int, string, ...float] $value): void {}\n",
        "function a(vec[int, string, ...float] $value): void {}\n",
    );
    assert_formats(
        "function b(vec[int, ...] $value): void {}\n",
        "function b(vec[int, ...] $value): void {}\n",
    );
    assert_formats(
        "function c(vec[...bool] $value): void {}\n",
        "function c(vec[...bool] $value): void {}\n",
    );
    assert_formats(
        "function d((int, string, ...float) $tuple): void {}\n",
        "function d((int, string, ...float) $tuple): void {}\n",
    );
    assert_formats(
        "function e(dict['id' => int, ...<string, float|int>] $record): void {}\n",
        "function e(dict['id' => int, ...<string, float|int>] $record): void {}\n",
    );
}

#[test]
fn access_and_calls() {
    assert_formats("$x = Foo::BAR;\n", "$x = Foo::BAR;\n");
    assert_formats("$x = Foo::method();\n", "$x = Foo::method();\n");
    assert_formats("$x = Foo::$prop;\n", "$x = Foo::$prop;\n");
    assert_formats("$x = $a->b()->c()->d();\n", "$x = $a->b()->c()->d();\n");
    assert_formats("foo(a: 1, b: 2);\n", "foo(\n  a: 1,\n  b: 2,\n);\n");
}

#[test]
fn calls_break_arguments_before_short_member_chains() {
    assert_formats(
        "$result=$this->performExchange($request,$configuration,$cancellation,$headers,$contentLength,$chunked,$target);",
        concat!(
            "$result = $this->performExchange(\n",
            "  $request,\n",
            "  $configuration,\n",
            "  $cancellation,\n",
            "  $headers,\n",
            "  $contentLength,\n",
            "  $chunked,\n",
            "  $target,\n",
            ");\n",
        ),
    );
    assert_formats(
        "final class Pool {public function close():void{$this->lease->release(!$connection->isClosed()&&$connection->isReusable());}}",
        concat!(
            "final class Pool {\n",
            "  public function close(): void {\n",
            "    $this->lease->release(\n",
            "      !$connection->isClosed()\n",
            "      && $connection->isReusable(),\n",
            "    );\n",
            "  }\n",
            "}\n",
        ),
    );
    assert_formats(
        "$ordering=$this->date->compare($other->date)->then($this->time->compare($other->time));",
        concat!(
            "$ordering = $this->date->compare($other->date)->then(\n",
            "  $this->time->compare($other->time),\n",
            ");\n",
        ),
    );
    assert_formats(
        "$cookie=SetCookie::fromParts($name,$value,$expires,$maximumAge,$domain,$path,$secure,$httpOnly,$sameSite);",
        concat!(
            "$cookie = SetCookie::fromParts(\n",
            "  $name,\n",
            "  $value,\n",
            "  $expires,\n",
            "  $maximumAge,\n",
            "  $domain,\n",
            "  $path,\n",
            "  $secure,\n",
            "  $httpOnly,\n",
            "  $sameSite,\n",
            ");\n",
        ),
    );
}

#[test]
fn calls_hug_short_closures_without_changing_body_indentation() {
    assert_formats(
        "$id=$token->register(function () use ($weak):void {$linked=$weak->get();});",
        concat!(
            "$id = $token->register(function() use ($weak): void {\n",
            "  $linked = $weak->get();\n",
            "});\n",
        ),
    );
    assert_formats(
        "final class Gate {public function wait():void{$id=$cancellation->register(function () use ($cancellation,$waiter):void {$waiter->active=false;});}}",
        concat!(
            "final class Gate {\n",
            "  public function wait(): void {\n",
            "    $id = $cancellation->register(\n",
            "      function() use ($cancellation, $waiter): void {\n",
            "        $waiter->active = false;\n",
            "      },\n",
            "    );\n",
            "  }\n",
            "}\n",
        ),
    );
}

#[test]
fn calls_hug_a_single_breaking_argument() {
    assert_formats(
        concat!(
            "assert!(Dict\\flatten::<string,int>(vec[dict['a'=>1],",
            "dict['b'=>2,'a'=>10]])==dict['a'=>10,'b'=>2]);",
        ),
        concat!(
            "assert!(\n",
            "  Dict\\flatten::<string, int>(vec[\n",
            "    dict['a' => 1],\n",
            "    dict['b' => 2, 'a' => 10],\n",
            "  ]) == dict['a' => 10, 'b' => 2],\n",
            ");\n",
        ),
    );
    assert_formats(
        "consume(match($value){1=>'one',$_=>'other'});",
        concat!(
            "consume(match ($value) {\n",
            "  1 => 'one',\n",
            "  $_ => 'other',\n",
            "});\n",
        ),
    );
}

#[test]
fn a_method_call_attaches_to_a_multiline_instantiation() {
    assert_formats(
        "$stream=(new TLS\\Connector::<TEndpoint>($configuration->tls->withAlpnProtocols(alpn_protocols($versions))))->connect($stream,$serverName,$cancellation);",
        concat!(
            "$stream = new TLS\\Connector::<TEndpoint>(\n",
            "  $configuration->tls->withAlpnProtocols(alpn_protocols($versions)),\n",
            ")->connect($stream, $serverName, $cancellation);\n",
        ),
    );
}

#[test]
fn every_spine_shape_uses_the_same_renderer() {
    let source = concat!(
        "foo(1,2);make()(1,2);",
        "Foo::bar(1,2);$class::bar(1,2);",
        "$a=Foo::BAR;$b=$class::BAR;",
        "Foo::$field=1;$class::$field=2;$items[0]=3;",
        "$first=foo(?,2);$second=make()(?,2);",
    );
    let expected = concat!(
        "foo(1, 2);\n",
        "make()(1, 2);\n",
        "Foo::bar(1, 2);\n",
        "$class::bar(1, 2);\n",
        "$a = Foo::BAR;\n",
        "$b = $class::BAR;\n",
        "Foo::$field = 1;\n",
        "$class::$field = 2;\n",
        "$items[0] = 3;\n",
        "$first = foo(?, 2);\n",
        "$second = make()(?, 2);\n",
    );

    assert_formats(source, expected);
    assert_idempotent("Foo::bar(/* argument */ 1); $class::BAR /* trailing */;");
}

#[test]
fn first_class_callables_and_partial_application() {
    assert_idempotent("foo(...);");
    assert_idempotent("$f = strlen(...);");
    assert_idempotent("$p = foo(1, ...);");
    assert_idempotent("$g = foo(?, 1);");
    assert_idempotent("$h = $obj->method(?);");
}

#[test]
fn attributes_with_arguments() {
    assert_idempotent("#[Route('/x', method: 'GET')]\nclass C {}");
    assert_idempotent("#[A, B(1)]\nfunction f(): void {}");
}

#[test]
fn interface_with_abstract_methods() {
    let source = "interface Repo extends Base { public function find(int $id): null|Entity; }";
    let expected = concat!(
        "interface Repo extends Base {\n",
        "  public function find(int $id): null|Entity;\n",
        "}\n",
    );
    assert_formats(source, expected);
}

#[test]
fn sealed_interface_permissions() {
    assert_formats(
        "interface Result extends Outcome for Success,Failure{}",
        "interface Result extends Outcome for Success, Failure {}\n",
    );
}

#[test]
fn sealed_class_permissions() {
    assert_formats(
        "abstract class Event extends Base implements Recorded for Login,Logout{}",
        "abstract class Event extends Base implements Recorded for Login, Logout {}\n",
    );
}

#[test]
fn tuple_destructuring_assignment() {
    assert_idempotent("($a, $b) = $pair;");
    assert_idempotent("($a, $b[0], $c->k) = $value;");
    assert_formats(
        "($given,$family='(none)',...$rest)=$parts;",
        "($given, $family = '(none)', ...$rest) = $parts;\n",
    );
}

#[test]
fn dictionary_destructuring_targets() {
    assert_formats(
        "dict['id'=>$id,'profile'=>dict['name'=>$name]]= $row;",
        "dict['id' => $id, 'profile' => dict['name' => $name]] = $row;\n",
    );
    assert_formats(
        "$value=match($row){dict['id'=>$id @ mixed,...]=>$id,$_=>null,};",
        concat!(
            "$value = match ($row) {\n",
            "  dict['id' => $id @ mixed, ...] => $id,\n",
            "  $_ => null,\n",
            "};\n",
        ),
    );
}

#[test]
fn unary_and_coalesce_and_bitwise() {
    assert_idempotent("$x = !$a;");
    assert_idempotent("$y = -$b;");
    assert_idempotent("$z = ~$c;");
    assert_idempotent("$w = $i++;");
    assert_idempotent("$v = ++$i;");
    assert_idempotent("$c = $a ?? $b ?? $d;");
    assert_idempotent("$f = $flags & MASK;");
    assert_idempotent("$s = 'a' . $b . 'c';");
    assert_idempotent("$p = ($a + $b) == $d;");
}

#[test]
fn throw_and_constructs() {
    assert_idempotent("$t = $x ?? throw new Boom();");
    assert_formats("$c = clone!($o);\n", "$c = clone!($o);\n");
    assert_formats(
        "$d = clone!($o, name: 'x', age: 1);\n",
        "$d = clone!($o, name: 'x', age: 1);\n",
    );
    assert_formats("exit!(1);\n", "exit!(1);\n");
    assert_formats("exit!();\n", "exit!();\n");
    assert_formats("panic!(\"impossible\",);\n", "panic!('impossible');\n");
    assert_idempotent("panic!(/* before */ 'impossible' /* after */);");
    assert_formats(
        "require!('src/bootstrap');\n",
        "require!('src/bootstrap');\n",
    );
    assert_formats(
        "require_once!('src/helpers');\n",
        "require_once!('src/helpers');\n",
    );
    assert_formats("$n = length!($v);\n", "$n = length!($v);\n");
    assert_formats(
        "write_line!('hello', $name);\n",
        "write_line!('hello', $name);\n",
    );
    assert_formats("$s = remove!($v, 0);\n", "$s = remove!($v, 0);\n");
    assert_formats(
        "$s=swap_remove!($values,$index,);\n",
        "$s = swap_remove!($values, $index);\n",
    );
    assert_idempotent("debug!($x, $y);");
}

#[test]
fn file_and_directory_constructs() {
    assert_formats("$p = file!();\n", "$p = file!();\n");
    assert_formats("$d = directory!();\n", "$d = directory!();\n");
    assert_formats(
        "$template=embed!(\"./template.tpl\",);\n",
        "$template = embed!('./template.tpl');\n",
    );
    assert_idempotent("write_line!(file!(), directory!());");
    assert_idempotent("$template = embed!('./template.tpl');");
    assert_idempotent("$template = embed!(/* before */ './template.tpl' /* after */);");
}

#[test]
fn nested_arrays_and_keys() {
    assert_idempotent("$x = dict['a' => vec[1, 2], 'b' => 3];");
    assert_idempotent("$t = (1, 'two', vec[3]);");
    assert_formats("$s = (1,);\n", "$s = (1,);\n");
}

#[test]
fn keywords_used_as_names_round_trip() {
    assert_idempotent("const enum = 'hello';");
    assert_idempotent("$x = out;");
    assert_idempotent("$n = int($x);");
    assert_idempotent("function vec(): int {}");
    assert_idempotent("class C { function match(): int {} const enum = 1; }");
}

#[test]
fn long_call_breaks_arguments() {
    let source = concat!(
        "register('alpha', 'bravo', 'charlie', 'delta', 'echo', 'foxtrot', 'golf', ",
        "'hotel', 'india', 'juliet', 'kilo', 'lima');\n",
    );
    let expected = concat!(
        "register(\n",
        "  'alpha',\n",
        "  'bravo',\n",
        "  'charlie',\n",
        "  'delta',\n",
        "  'echo',\n",
        "  'foxtrot',\n",
        "  'golf',\n",
        "  'hotel',\n",
        "  'india',\n",
        "  'juliet',\n",
        "  'kilo',\n",
        "  'lima',\n",
        ");\n",
    );
    assert_formats(source, expected);
}

#[test]
fn expression_bodied_short_closure_with_return_type() {
    assert_formats("$f = fn($x): int => $x;\n", "$f = fn($x): int => $x;\n");
    assert_idempotent("$f = fn() => (/* keep */ $x);");
    assert_formats(
        concat!(
            "$assertion_value_3=fn():dict<string|int|bool,mixed> =>",
            "Whim\\_Private\\dict_from_entries(vec[(vec[],1)]);",
        ),
        concat!(
            "$assertion_value_3 = fn(): dict<string|int|bool, mixed> => (\n",
            "  Whim\\_Private\\dict_from_entries(vec[(vec[], 1)])\n",
            ");\n",
        ),
    );
}

#[test]
fn braced_namespace() {
    let source = "namespace App { const X = 1; }";
    let expected = "namespace App {\n  const X = 1;\n}\n";
    assert_formats(source, expected);
}

#[test]
fn function_type_alias() {
    assert_formats(
        "type Handler = fn(string,int,=float): string;",
        "type Handler = fn(string, int, =float): string;\n",
    );
    assert_formats("type Any = fn;", "type Any = fn;\n");
}

#[test]
fn array_vec_dict_and_tuple_types() {
    assert_formats(
        "type Collection = array<int,string>;",
        "type Collection = array<int, string>;\n",
    );
    assert_formats("type A = vec<int>;", "type A = vec<int>;\n");
    assert_formats(
        "type B = dict<string,int>;",
        "type B = dict<string, int>;\n",
    );
    assert_formats(
        "type C = (int, string, bool);",
        "type C = (int, string, bool);\n",
    );
    assert_formats("type D = (int,);", "type D = (int,);\n");
    assert_formats("type E = (int);", "type E = (int);\n");
    assert_formats("type F=(...);", "type F = (...);\n");
    assert_formats(
        "type G=(int,string,...bool);",
        "type G = (int, string, ...bool);\n",
    );
}

#[test]
fn parameters_break_before_a_return_tuple() {
    assert_formats(
        "function bounded_with_capacity<T>(PositiveInt $capacity): (Sender<T>, Receiver<T>) {}",
        concat!(
            "function bounded_with_capacity<T>(\n",
            "  PositiveInt $capacity,\n",
            "): (Sender<T>, Receiver<T>) {}\n",
        ),
    );
}

#[test]
fn empty_parameters_break_before_the_return_type() {
    assert_formats(
        "class Family { public function ianaFamily(): 1|2 { return match ($this) { Family::V4 => 1, Family::V6 => 2, }; } }",
        concat!(
            "class Family {\n",
            "  public function ianaFamily(): 1|2 {\n",
            "    return match ($this) {\n",
            "      Family::V4 => 1,\n",
            "      Family::V6 => 2,\n",
            "    };\n",
            "  }\n",
            "}\n",
        ),
    );
    assert_formats(
        "interface Endpoint { public function takeTransmit(): null|(string,string,int,null|string,int) {} }",
        concat!(
            "interface Endpoint {\n",
            "  public function takeTransmit(\n",
            "  ): null|(string, string, int, null|string, int) {}\n",
            "}\n",
        ),
    );
}

#[test]
fn parameters_break_before_the_return_type() {
    assert_formats(
        "function dict_sort(mixed $values,mixed $comparator):dict<string|int|bool,mixed>{}",
        concat!(
            "function dict_sort(\n",
            "  mixed $values,\n",
            "  mixed $comparator,\n",
            "): dict<string|int|bool, mixed> {}\n",
        ),
    );
}

#[test]
fn class_extends_stays_flat_before_implements_breaks() {
    assert_formats(
        "final class ServerConnection extends _Private\\ConnectionImpl implements Connection {}",
        concat!(
            "final class ServerConnection extends _Private\\ConnectionImpl implements\n",
            "  Connection {}\n",
        ),
    );
}

#[test]
fn negated_types() {
    assert_formats("type T=!A&B|!(C|D);", "type T = !A&B|!(C|D);\n");
    assert_formats(
        "function accept(!null $value):string&!''{return $value;}",
        "function accept(!null $value): string&!'' {\n  return $value;\n}\n",
    );
}

#[test]
fn classname_types() {
    assert_formats("type A = classname<User>;", "type A = classname<User>;\n");
    assert_formats(
        "type C = dict<string,classname<User>>;",
        "type C = dict<string, classname<User>>;\n",
    );
}

#[test]
fn generic_type_alias() {
    assert_formats("type Pair<A,B> = (A,B);", "type Pair<A, B> = (A, B);\n");
}

#[test]
fn generic_newtype() {
    assert_formats(
        "#[Domain]newtype Identifier<T:int|string=int>=T;",
        "#[Domain]\nnewtype Identifier<T: int|string = int> = T;\n",
    );
}

#[test]
fn generic_class_with_variance_bound_and_default() {
    assert_formats(
        "class Box<in T:object=mixed,out U> extends Base<T> implements Countable<int> {}",
        "class Box<in T: object = mixed, out U> extends Base<T> implements\n  Countable<int> {}\n",
    );
}

#[test]
fn generic_function_and_named_type_arguments() {
    assert_formats(
        "function map<T,U>(fn(T):U $f,Vector<T> $xs):Vector<U>{return $xs;}",
        "function map<T, U>(fn(T): U $f, Vector<T> $xs): Vector<U> {\n  return $xs;\n}\n",
    );
}

#[test]
fn turbofish_calls() {
    assert_formats(
        "$a = make::<int,string>();\n",
        "$a = make::<int, string>();\n",
    );
    assert_formats("Vector::<int>::new();\n", "Vector::<int>::new();\n");
    assert_formats("$b = $obj->get::<T>();\n", "$b = $obj->get::<T>();\n");
    assert_formats("$c = new Box::<int>();\n", "$c = new Box::<int>();\n");
}

#[test]
fn kitchen_sink_is_idempotent() {
    let source = concat!(
        "#!/usr/bin/env whim\n",
        "namespace App;\n",
        "use App\\{Logger, Cache as C};\n",
        "type Id = int|string;\n",
        "const VERSION = '1.0';\n",
        "#[Entity]\n",
        "final class Service extends Base implements Runnable {\n",
        "    public const int LIMIT = 10;\n",
        "    private null|Logger $logger = null;\n",
        "    public function run(array $items): void {\n",
        "        foreach ($items as $i => $item) {\n",
        "            if ($item is int) {\n",
        "                $this->logger?->log($item);\n",
        "            } else {\n",
        "                throw new Invalid('bad');\n",
        "            }\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    assert_idempotent(source);
}

#[test]
fn settings_reject_a_zero_or_absurd_print_width() {
    let zero = FormatSettings {
        print_width: 0,
        ..FormatSettings::default()
    };
    let error = zero
        .validate()
        .expect_err("a print width of zero leaves no room for any line");
    assert_eq!(error.setting, "print_width");

    let absurd = FormatSettings {
        print_width: 1_000_000,
        ..FormatSettings::default()
    };
    absurd
        .validate()
        .expect_err("a print width past the maximum is rejected");
}

#[test]
fn settings_reject_a_zero_or_absurd_tab_width() {
    let zero = FormatSettings {
        tab_width: 0,
        ..FormatSettings::default()
    };
    let error = zero
        .validate()
        .expect_err("a tab width of zero makes indentation invisible");
    assert_eq!(error.setting, "tab_width");

    let absurd = FormatSettings {
        tab_width: 100_000,
        ..FormatSettings::default()
    };
    absurd
        .validate()
        .expect_err("a tab width past the maximum is rejected");
}

#[test]
fn settings_reject_a_tab_width_wider_than_the_print_width() {
    let settings = FormatSettings {
        print_width: 20,
        tab_width: 24,
        ..FormatSettings::default()
    };
    let error = settings
        .validate()
        .expect_err("one indentation level would overflow every line");
    assert_eq!(error.setting, "tab_width");
    assert!(
        error.to_string().contains("print_width"),
        "the message should name the setting it conflicts with: {error}"
    );
}

#[test]
fn the_default_settings_are_valid() {
    FormatSettings::default()
        .validate()
        .expect("the defaults are usable");
}

#[test]
fn carriage_return_line_endings_are_normalised() {
    let line_feed = "$a = 1;\n\n$b = 2;\n";
    let carriage_return_line_feed = "$a = 1;\r\n\r\n$b = 2;\r\n";
    let classic_mac = "$a = 1;\r\r$b = 2;\r";

    let expected = format(line_feed);
    assert_eq!(
        format(carriage_return_line_feed),
        expected,
        "`\\r\\n` should read as one line break, not two"
    );
    assert_eq!(
        format(classic_mac),
        expected,
        "a lone `\\r` should read as a line break"
    );
}

#[test]
fn a_blank_line_written_with_carriage_returns_is_still_a_blank_line() {
    assert_formats("$a = 1;\r\n\r\n$b = 2;\r\n", "$a = 1;\n\n$b = 2;\n");
    assert_formats("$a = 1;\r\n$b = 2;\r\n", "$a = 1;\n$b = 2;\n");
}

#[test]
fn a_block_comment_keeps_its_interior_indentation() {
    let source = "/*\n * A table:\n *\n *     name    kind\n *     ----    ----\n */\n$a = 1;\n";
    assert_formats(
        source,
        "/*\n * A table:\n *\n *     name    kind\n *     ----    ----\n */\n$a = 1;\n",
    );
}

#[test]
fn a_block_comment_star_column_is_realigned_and_other_lines_keep_their_shape() {
    assert_formats(
        "class Holder {\n/*\n     * misaligned\n         indented text\n */\npublic int $x = 1;\n}\n",
        "class Holder {\n  /*\n   * misaligned\n           indented text\n   */\n  public int $x = 1;\n}\n",
    );
}

#[test]
fn a_block_comment_at_the_top_level_keeps_its_exact_interior() {
    let source = "/*\n * A table:\n *\n *     name    kind\n *     ----    ----\n */\n$a = 1;\n";
    assert_formats(source, source);
}

#[test]
fn a_block_comment_written_with_carriage_returns_keeps_its_lines() {
    assert_formats(
        "/*\r\n * first\r\n * second\r\n */\r\n$a = 1;\r\n",
        "/*\n * first\n * second\n */\n$a = 1;\n",
    );
}

#[test]
fn display_width_counts_terminal_columns_not_scalar_values() {
    use crate::printer::string_width;

    assert_eq!(string_width("abc"), 3);
    assert_eq!(
        string_width("日本語"),
        6,
        "an East Asian wide character occupies two columns"
    );
    assert_eq!(
        string_width("😀"),
        2,
        "an emoji occupies two columns even though it is one scalar value"
    );
    assert_eq!(
        string_width("e\u{0301}"),
        1,
        "a combining mark occupies no column of its own"
    );
    assert_eq!(
        string_width("a\nb\u{4e2d}"),
        3,
        "width is measured on the last line"
    );
}

#[test]
fn a_line_of_wide_characters_breaks_at_the_column_it_reaches() {
    let arena = LocalArena::new();
    let settings = FormatSettings {
        print_width: 20,
        ..FormatSettings::default()
    };
    let source = "$a = compute('日本語日本語', '日本語日本語');\n";
    let formatted = format_source(&arena, source, settings).expect("source should parse");

    assert!(
        formatted.contains('\n') && formatted.lines().count() > 1,
        "twelve wide characters exceed a print width of twenty columns, so the \
         call should break:\n{formatted}"
    );
}

/// The most `link` repetitions the structural limit admits after `head`, the
/// per-link cost measured between the first and second link rather than
/// assumed.
fn links_within_limit(head: &str, link: &str) -> usize {
    let depth = |links: usize| {
        let mut source = String::from(head);
        for _ in 0..links {
            source.push_str(link);
        }
        source.push(';');

        let arena = LocalArena::new();
        let program = parse(&arena, &source).expect("a measuring chain parses");

        deepest_path(Node::Program(program)).levels
    };
    let one = depth(1);
    let step = depth(2) - one;

    1 + (MAX_STRUCTURAL_DEPTH - one) / step
}

#[test]
fn a_long_flat_operator_chain_formats_without_recursing() {
    let links = links_within_limit("$x = 0", "+1");
    Builder::new()
        .stack_size(512 * 1024)
        .spawn(move || {
            let mut source = String::from("$x = 0");
            for _ in 0..links {
                source.push_str("+1");
            }
            source.push_str(";\n");

            let once = format(&source);
            let twice = format(&once);
            assert_eq!(once, twice, "formatting is not idempotent");
        })
        .expect("spawn")
        .join()
        .expect("no stack overflow");
}

#[test]
fn a_long_mixed_postfix_chain_formats_without_recursing() {
    let links = links_within_limit("$y = $x", "[0]?->a");
    let formatted = Builder::new()
        .stack_size(512 * 1024)
        .spawn(move || {
            let mut source = String::from("$y = $x");
            for _ in 0..links {
                source.push_str("[0]?->a");
            }
            source.push_str(";\n");

            let once = format(&source);
            let twice = format(&once);
            assert_eq!(once, twice, "formatting is not idempotent");

            once
        })
        .expect("spawn")
        .join()
        .expect("no stack overflow");

    let mut expected = String::from("$y = $x[0]\n  ?->a");
    for _ in 1..links {
        expected.push_str("[0]\n  ?->a");
    }
    expected.push_str(";\n");
    assert_eq!(formatted, expected);
}

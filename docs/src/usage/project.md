# A Small Program

This program counts the words in a file. It uses command-line arguments, file
I/O, functions, loops, and a dict.

Save it as `words.whim`:

```whim,norun
use Whim\Env;
use Whim\File;
use Whim\Str;

function count_words(string $text): dict<string, int> {
  $counts = dict[];
  $words = Str\split(Str\lowercase($text), ' ');

  foreach ($words as $word) {
    $word = Str\trim($word);
    if ($word == '') {
      continue;
    }

    if (!contains_key!($counts, $word)) {
      $counts[$word] = 0;
    }

    $counts[$word]++;
  }

  return $counts;
}

$arguments = Env\get_arguments();
if (length!($arguments) < 1) {
  write_error_line!('usage: whim words.whim <file>');
  exit!(2);
}

$text = File\read($arguments[0]);
$counts = count_words($text);

foreach ($counts as $word => $count) {
  write_line!($word . ': ' . $count);
}
```

Run it:

```console
whim words.whim README.md
```

`Env\get_arguments()` contains the arguments after the source file. The input
path is at index `0`.

`dict[]` creates an empty dict. Reading a missing key throws, so the program
uses `contains_key!` before it reads the count. The first word starts at zero,
then `++` adds one.

Dicts keep insertion order. The last loop prints words in the order in which
the program first found them.

This parser splits only on spaces. A full word parser would also handle tabs,
newlines, and punctuation. This example keeps the parser short so it can focus
on the language.

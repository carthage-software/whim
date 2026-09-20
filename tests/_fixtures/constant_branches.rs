use std::env::consts;

pub(super) fn bodies() -> Vec<String> {
    let mut bodies = vec![
        "if ('linux' == 'linux') { return 'chosen'; } else { return 'discarded'; }".to_string(),
        "if ('linux' != 'linux') { return 'discarded'; } return 'chosen';".to_string(),
        "if (false) { return 'discarded'; } if (true) { return 'chosen'; } return 'discarded';".to_string(),
        "if (1 == 1.0) { return 'discarded'; } return 'chosen';".to_string(),
        "if (9007199254740993 > 9007199254740992.0) { return 'chosen'; } return 'discarded';".to_string(),
        "return match ('linux') { 'macos' => 'discarded', 'linux' => 'chosen', 'linux' => 'discarded', _ => 'discarded' };".to_string(),
        "return match ('b') { 'a' => 'discarded', 'b' => 'chosen', 'c' => 'discarded', _ => 'discarded' };".to_string(),
        "return match (2) { 1 => 'discarded', 2 => 'chosen', 3 => 'discarded', _ => 'discarded' };".to_string(),
        "return match (-0.0) { 0.0 => 'chosen', 1.5 => 'discarded', _ => 'discarded' };".to_string(),
        "return match (false) { true => 'discarded', false => 'chosen' };".to_string(),
        "return match ('other') { 'linux' => 'discarded', 'macos' => 'discarded', _ => 'chosen' };".to_string(),
        format!("if (operating_system!() == '{}') {{ return 'chosen'; }} return 'discarded';", consts::OS),
        format!("return match (shared_library_suffix!()) {{ '{}' => 'chosen', _ => 'discarded' }};", consts::DLL_SUFFIX),
        format!("return match (operating_system!() . '/' . cpu_architecture!()) {{ '{}/{}' => 'chosen', _ => 'discarded' }};", consts::OS, consts::ARCH),
        "return match (operating_system!()) { $os @ string => match ($os == operating_system!()) { true => 'chosen', _ => 'discarded' } };".to_string(),
        "if ((('li' . 'nux') == 'linux' && length!(operating_system!()) > 0) || false) { return 'chosen'; } return 'discarded';".to_string(),
        "$value = match (operating_system!() == operating_system!()) { true => 'linux', _ => 'macos' }; return match ($value) { 'linux' => 'chosen', _ => 'discarded' };".to_string(),
        format!("return match ((operating_system!(), cpu_architecture!())) {{ ('{}', '{}') => 'chosen', _ => 'discarded' }};", consts::OS, consts::ARCH),
        "return match ((1, 'linux')) { (2, string) => 'discarded', (1..=3, ...string) => 'chosen', _ => 'discarded' };".to_string(),
        "return match (length!(operating_system!())) { 0 => 'discarded', 1.. => 'chosen', _ => 'discarded' };".to_string(),
        "return match (null) { true => 'discarded', false => 'discarded', _ => 'chosen' };".to_string(),
    ];
    for (construct, expected) in [
        ("cpu_architecture", consts::ARCH),
        ("operating_system", consts::OS),
        ("operating_system_family", consts::FAMILY),
        ("shared_library_prefix", consts::DLL_PREFIX),
        ("shared_library_suffix", consts::DLL_SUFFIX),
        ("shared_library_extension", consts::DLL_EXTENSION),
        ("executable_suffix", consts::EXE_SUFFIX),
        ("executable_extension", consts::EXE_EXTENSION),
    ] {
        bodies.push(format!(
            "if ({construct}!() == '{expected}') {{ return 'chosen'; }} return 'discarded';"
        ));
        bodies.push(format!(
            "return match ({construct}!()) {{ '{expected}' => 'chosen', _ => 'discarded' }};"
        ));
    }
    bodies
}

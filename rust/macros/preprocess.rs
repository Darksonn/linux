use proc_macro::{TokenStream, TokenTree};


static PREPROCESS: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn parse_string(tree: TokenTree) -> String {
    let mut lit = match tree {
        TokenTree::Literal(lit) => lit.to_string(),
        _ => panic!(),
    };

    if lit.chars().next() == Some('"') {
        // normal string
        let mut res = String::new();

        let mut iter = lit[1..lit.len()-1].chars();
        loop {
            match iter.next() {
                None => break,
                Some('\\') => match iter.next().unwrap() {
                    '\\' => res.push('\\'),
                    '"' => res.push('"'),
                    'n' => res.push('\n'),
                    't' => res.push('\t'),
                    'r' => res.push('\r'),
                    e => panic!("Unsupported escape: {e}"),
                },
                Some(c) => res.push(c),
            }
        }
        res
    } else {
        // raw string
        let mut count = 0;
        for c in lit.chars().rev() {
            if c != '#' { break; }
            count += 1;
        }
        lit[count+1 .. lit.len()-count].to_string()
    }
}

/// dox
pub(crate) fn preprocess(input: TokenStream) -> TokenStream {
    use std::process::Command;

    let _lock = PREPROCESS.lock();
    let inp = "./preprocess.c";
    let out = "./preprocess.i";

    let mut tokens = input.into_iter();
    let first = parse_string(tokens.next().unwrap());
    let comma = tokens.next().unwrap();
    let second = parse_string(tokens.next().unwrap());
    assert!(tokens.next().is_none());

    let contents = format!("{first}\nRUST_PREPROCESS_DIVIDER\n{second}\n");

    std::fs::write(inp, contents).unwrap();

    let cpp = std::env::var("CPP").expect("Can't find CPP in environment.");
    let cppflags = std::env::var("KBUILD_CPPFLAGS").expect("Can't find CPPFLAGS in environment.");
    let cflags = std::env::var("KBUILD_CFLAGS").expect("Can't find CFLAGS in environment.");
    let include = std::env::var("LINUXINCLUDE").expect("Can't find LINUXINCLUDE in environment.");

    let command = format!("{cpp} {cppflags} {cflags} {include} -o {out} {inp}");

    let status = Command::new("sh")
        .arg("-c")
        .arg(command)
        .status()
        .unwrap();

    assert!(status.success());

    let mut output = String::new();
    let mut found_divider = false;
    for line in std::fs::read_to_string(out).unwrap().lines() {
        if found_divider {
            output.push_str(line);
        } else if line == "RUST_PREPROCESS_DIVIDER" {
            found_divider = true;
        }
    }

    proc_macro::TokenTree::Literal(proc_macro::Literal::string(&output)).into()
}

#!/usr/bin/env bash
# Generates a small demo git repository with varied file types and changes so the
# git-review apps have something interesting to display. Safe to re-run.
#
# Usage: tools/make-sample-repo.sh [target-dir]   (default: /tmp/git-review-sample)
set -euo pipefail

DIR="${1:-/tmp/git-review-sample}"
rm -rf "$DIR"
mkdir -p "$DIR"
cd "$DIR"

git init -q
git config user.name  "Ada Lovelace"
git config user.email "ada@example.com"
git config commit.gpgsign false

commit() { GIT_AUTHOR_DATE="$1" GIT_COMMITTER_DATE="$1" git commit -q -m "$2"; }

# --- commit 1: initial project ---------------------------------------------
mkdir -p src
cat > src/main.rs <<'EOF'
fn main() {
    let name = "world";
    println!("Hello, {name}!");
}
EOF
cat > README.md <<'EOF'
# Demo

A tiny sample project used to exercise the git-review apps.
EOF
git add -A
commit "2026-01-04T09:00:00" "Initial commit: hello world"

# --- commit 2: add a module + python script --------------------------------
cat > src/math.rs <<'EOF'
/// Sum of the first `n` natural numbers.
pub fn triangular(n: u64) -> u64 {
    (0..=n).sum()
}

pub fn fib(n: u32) -> u64 {
    let (mut a, mut b) = (0u64, 1u64);
    for _ in 0..n {
        (a, b) = (b, a + b);
    }
    a
}
EOF
mkdir -p scripts
cat > scripts/stats.py <<'EOF'
#!/usr/bin/env python3
"""Print basic stats about a list of numbers."""
import sys


def main(args):
    nums = [float(a) for a in args]
    print("count", len(nums))
    print("sum", sum(nums))
    print("mean", sum(nums) / len(nums) if nums else 0)


if __name__ == "__main__":
    main(sys.argv[1:])
EOF
git add -A
commit "2026-02-12T14:30:00" "Add math module and a python stats script"

# --- commit 3: modify main, use the module --------------------------------
cat > src/main.rs <<'EOF'
mod math;

fn main() {
    let name = "git-review";
    println!("Hello, {name}!");

    for n in 1..=5 {
        println!("triangular({n}) = {}", math::triangular(n));
    }
    println!("fib(10) = {}", math::fib(10));
}
EOF
git add -A
commit "2026-03-01T11:05:00" "Wire up the math module in main"

# --- commit 4: whitespace-only reformat (to test 'show space changes') -----
cat > src/math.rs <<'EOF'
/// Sum of the first `n` natural numbers.
pub fn triangular(n: u64) -> u64 {
        (0..=n).sum()
}

pub fn fib(n: u32) -> u64 {
        let (mut a, mut b) = (0u64, 1u64);
        for _ in 0..n {
                (a, b) = (b, a + b);
        }
        a
}
EOF
git add -A
commit "2026-03-02T08:15:00" "Reformat math.rs (whitespace only)"

# --- commit 5: add config + delete README, add a longer file ---------------
cat > config.toml <<'EOF'
[app]
name = "demo"
theme = "light"

[window]
width = 1200
height = 800
EOF
rm README.md
cat > src/lib.rs <<'EOF'
//! Demo library with a few odds and ends.

pub mod math {
    pub fn square(x: i64) -> i64 { x * x }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    Circle { r: f64 },
    Rect { w: f64, h: f64 },
}

impl Shape {
    pub fn area(&self) -> f64 {
        match self {
            Shape::Circle { r } => std::f64::consts::PI * r * r,
            Shape::Rect { w, h } => w * h,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rect_area() {
        assert_eq!(Shape::Rect { w: 2.0, h: 3.0 }.area(), 6.0);
    }
}
EOF
git add -A
commit "2026-04-10T16:45:00" "Add config.toml and lib.rs, drop README"

echo "Sample repo ready at: $DIR"
git -C "$DIR" --no-pager log --oneline

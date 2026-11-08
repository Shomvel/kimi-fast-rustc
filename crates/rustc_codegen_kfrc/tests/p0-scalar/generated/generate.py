"""Random scalar Rust programs for the P0 differential test.

Writes `lib.rs` (functions compiled by the backend under test) and
`driver.rs` (calls each function on fixed pseudo-random inputs and prints the
results). Programs are free of undefined behaviour and panics under
`-Coverflow-checks=off`: arithmetic wraps, shift amounts are masked, there is
no division, and every loop is bounded by a counter.

usage: python3 -I generate.py <seed> <function count> <output dir>
"""

import os
import random
import sys

INTS = ["i8", "u8", "i16", "u16", "i32", "u32", "i64", "u64", "isize", "usize"]
BITS = {"i8": 8, "u8": 8, "i16": 16, "u16": 16, "i32": 32, "u32": 32, "i64": 64, "u64": 64, "isize": 64, "usize": 64}


def literal(rng, ty):
    if ty == "bool":
        return rng.choice(["true", "false"])
    bits = BITS[ty]
    signed = ty.startswith("i")
    choice = rng.random()
    if choice < 0.3:
        value = rng.choice([0, 1, 2, 3, 7])
    elif choice < 0.5:
        value = (1 << (bits - 1)) - 1 if signed else (1 << bits) - 1
    else:
        value = rng.randrange(0, (1 << (bits - 1)) if signed else (1 << bits))
    if signed and rng.random() < 0.3:
        return f"(-{value}{ty})" if value != 0 else f"0{ty}"
    return f"{value}{ty}"


class Function:
    def __init__(self, rng, index, signatures):
        self.rng = rng
        self.index = index
        self.signatures = signatures  # earlier functions: (name, [param types], return type)
        self.params = [rng.choice(INTS + ["bool"]) for _ in range(rng.randint(1, 4))]
        self.ret = rng.choice(INTS + ["bool"])
        self.scopes = [[(f"p{i}", ty, False) for i, ty in enumerate(self.params)]]
        self.counter = 0
        self.loop_depth = 0

    def fresh(self, prefix):
        self.counter += 1
        return f"{prefix}{self.counter}"

    def variables(self, ty, mutable=None):
        found = []
        for scope in self.scopes:
            for name, var_ty, is_mut in scope:
                if var_ty == ty and (mutable is None or is_mut == mutable):
                    found.append(name)
        return found

    def expr(self, ty, depth):
        rng = self.rng
        if ty == "bool":
            return self.bool_expr(depth)
        candidates = self.variables(ty)
        if depth <= 0 or rng.random() < 0.2:
            if candidates and rng.random() < 0.7:
                return rng.choice(candidates)
            return literal(rng, ty)
        kind = rng.random()
        if kind < 0.35:
            op = rng.choice(["+", "-", "*", "&", "|", "^"])
            return f"({self.expr(ty, depth - 1)} {op} {self.expr(ty, depth - 1)})"
        if kind < 0.45:
            op = rng.choice(["<<", ">>"])
            amount_ty = rng.choice(INTS)
            return f"({self.expr(ty, depth - 1)} {op} {self.expr(amount_ty, depth - 1)})"
        if kind < 0.55:
            if ty.startswith("i") and rng.random() < 0.5:
                return f"(-{self.expr(ty, depth - 1)})"
            return f"(!{self.expr(ty, depth - 1)})"
        if kind < 0.68:
            source = rng.choice(INTS + ["bool"])
            return f"({self.expr(source, depth - 1)} as {ty})"
        if kind < 0.76:
            return f"(if {self.bool_expr(depth - 1)} {{ {self.expr(ty, depth - 1)} }} else {{ {self.expr(ty, depth - 1)} }})"
        if kind < 0.84:
            scrutinee_ty = rng.choice(["u8", "i32", "u64"])
            scrutinee = self.expr(scrutinee_ty, depth - 1)
            arms = [
                f"0 => {self.expr(ty, depth - 1)},",
                f"1 | 2 => {self.expr(ty, depth - 1)},",
                f"3..=9 => {self.expr(ty, depth - 1)},",
                f"_ => {self.expr(ty, depth - 1)},",
            ]
            return f"(match {scrutinee} & 15 {{ {' '.join(arms)} }})"
        if kind < 0.9 and self.signatures:
            name, params, ret = rng.choice(self.signatures)
            args = ", ".join(self.expr(param, depth - 1) for param in params)
            return f"({name}({args}) as {ty})" if ret != "bool" else f"({name}({args}) as {ty})"
        if kind < 0.95:
            label = self.fresh("'l")
            counter = self.fresh("n")
            limit = rng.randint(0, 6)
            value = self.expr(ty, depth - 1)
            return (
                f"({{ let mut {counter}: u32 = 0; {label}: loop {{ if {counter} >= {limit} {{ break {label} {value}; }} "
                f"{counter} += 1; }} }})"
            )
        return literal(rng, ty)

    def bool_expr(self, depth):
        rng = self.rng
        candidates = self.variables("bool")
        if depth <= 0 or rng.random() < 0.2:
            if candidates and rng.random() < 0.6:
                return rng.choice(candidates)
            return literal(rng, "bool")
        kind = rng.random()
        if kind < 0.5:
            ty = rng.choice(INTS)
            op = rng.choice(["<", "<=", ">", ">=", "==", "!="])
            return f"({self.expr(ty, depth - 1)} {op} {self.expr(ty, depth - 1)})"
        if kind < 0.7:
            op = rng.choice(["&&", "||"])
            return f"({self.bool_expr(depth - 1)} {op} {self.bool_expr(depth - 1)})"
        if kind < 0.8:
            return f"(!{self.bool_expr(depth - 1)})"
        if kind < 0.9:
            ty = rng.choice(INTS)
            return f"({self.expr(ty, depth - 1)} as u8 & 1 == 1)"
        return literal(rng, "bool")

    def statements(self, count, depth):
        lines = []
        for _ in range(count):
            lines.extend(self.statement(depth))
        return lines

    def block(self, count, depth):
        self.scopes.append([])
        lines = self.statements(count, depth)
        self.scopes.pop()
        return lines

    def statement(self, depth):
        rng = self.rng
        kind = rng.random()
        mutables = [(name, ty) for scope in self.scopes for name, ty, is_mut in scope if is_mut]
        if kind < 0.3 or not mutables:
            ty = rng.choice(INTS + ["bool"])
            name = self.fresh("v")
            line = f"let mut {name}: {ty} = {self.expr(ty, 3)};"
            self.scopes[-1].append((name, ty, True))
            return [line]
        if kind < 0.55:
            name, ty = rng.choice(mutables)
            if ty != "bool" and rng.random() < 0.5:
                op = rng.choice(["+=", "-=", "*=", "&=", "|=", "^="])
                return [f"{name} {op} {self.expr(ty, 2)};"]
            return [f"{name} = {self.expr(ty, 3)};"]
        if kind < 0.7 and depth > 0:
            condition = self.bool_expr(2)
            then_lines = self.block(rng.randint(1, 3), depth - 1)
            else_lines = self.block(rng.randint(0, 3), depth - 1)
            return [f"if {condition} {{"] + then_lines + ["} else {"] + else_lines + ["}"]
        if kind < 0.82 and depth > 0:
            counter = self.fresh("i")
            limit = rng.randint(0, 8)
            self.loop_depth += 1
            body = self.block(rng.randint(1, 3), depth - 1)
            self.loop_depth -= 1
            extra = []
            if rng.random() < 0.3:
                extra = [f"if {self.bool_expr(1)} {{ {counter} += 1; continue; }}"]
            if rng.random() < 0.2:
                extra.append(f"if {self.bool_expr(1)} {{ break; }}")
            return [f"let mut {counter}: u32 = 0;", f"while {counter} < {limit} {{"] + extra + body + [f"{counter} += 1;", "}"]
        if kind < 0.9 and depth > 0:
            scrutinee = self.expr(rng.choice(["u8", "i16", "u32"]), 2)
            arms = []
            for pattern in ["0", "1 | 5", "2..=4", "_"]:
                body = self.block(rng.randint(0, 2), depth - 1)
                arms.append(f"{pattern} => {{ " + " ".join(body) + " }")
            return [f"match {scrutinee} & 7 {{"] + arms + ["}"]
        if kind < 0.95:
            return [f"if {self.bool_expr(2)} {{ return {self.expr(self.ret, 2)}; }}"]
        name, ty = rng.choice(mutables)
        return [f"{name} = {self.expr(ty, 3)};"]

    def render(self):
        params = ", ".join(f"p{i}: {ty}" for i, ty in enumerate(self.params))
        body = self.statements(self.rng.randint(3, 8), 3)
        tail = self.expr(self.ret, 3)
        lines = ["#[inline(never)]", f"pub fn f{self.index}({params}) -> {self.ret} {{"]
        lines += ["    " + line for line in body]
        lines += [f"    {tail}", "}"]
        return "\n".join(lines)


def argument(rng, ty):
    if ty == "bool":
        return rng.choice(["true", "false"])
    bits = BITS[ty]
    value = rng.choice([0, 1, (1 << bits) - 1, 1 << (bits - 1), rng.getrandbits(bits)])
    return f"{value}u{bits} as {ty}" if bits < 128 else f"{value}"


def main():
    seed, count, out = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]
    rng = random.Random(seed)
    functions = []
    signatures = []
    for index in range(count):
        function = Function(rng, index, list(signatures))
        functions.append(function.render())
        signatures.append((f"f{index}", function.params, function.ret))
    os.makedirs(out, exist_ok=True)
    with open(os.path.join(out, "lib.rs"), "w") as handle:
        handle.write(f"// generated by generate.py seed={seed} count={count}\n")
        handle.write("#![allow(unused, unused_mut, unused_parens, overflowing_literals, unreachable_code, arithmetic_overflow, unused_comparisons, irrefutable_let_patterns)]\n\n")
        handle.write("\n\n".join(functions) + "\n")
    with open(os.path.join(out, "driver.rs"), "w") as handle:
        handle.write("#![allow(unused_parens, overflowing_literals)]\nuse generated::*;\n\nfn main() {\n")
        for name, params, _ in signatures:
            for _ in range(6):
                args = ", ".join(argument(rng, ty) for ty in params)
                handle.write(f'    println!("{name}({{:?}}) = {{:?}}", ({args},), {name}({args}));\n')
        handle.write("}\n")


if __name__ == "__main__":
    main()

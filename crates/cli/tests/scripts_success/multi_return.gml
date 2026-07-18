function test_ret_multi() {
    closure ret_multi(a, b, c) {
        return 1, 2, a, b, c;
    };

    let a, b, c, d, e = ret_multi(3, 4, 5);
    assert(a == 1, b == 2, c == 3, d == 4, e == 5);
}
test_ret_multi();

function test_ret_multi_try_catch() {
    closure ret_multi_try_catch(a, b, c) {
        try {
            return 5, 4, a, b, c;
        } catch(_) {
            assert(false);
        }
    }

    let a, b, c, d, e = ret_multi_try_catch(3, 2, 1);
    assert(a == 5, b == 4, c == 3, d == 2, e == 1);
}
test_ret_multi_try_catch();

function test_ret_multi_last_arg() {
    closure ret_multi2(a, b) {
        return a, b;
    }

    closure ret_multi3(a, b, c) {
        return a, b, c;
    }

    closure ret_multi5(a, b, c, d, e) {
        return a, b, c, d, e;
    }


    let a, b, c, d, e = ret_multi5(1, 2, ret_multi3(3, ret_multi2(4, 5)));
    assert(a == 1, b == 2, c == 3, d == 4, e == 5);
}
test_ret_multi_last_arg();

function test_ret_multi_last_ret() {
    closure ret_multi3(a, b, c) {
        return a, b, c;
    }

    closure ret_multi5(a, b, c, d, e) {
        return a, b, ret_multi3(c, d, e);
    }

    let a, b, c, d, e = ret_multi5(5, 4, 3, 2, 1);
    assert(a == 5, b == 4, c == 3, d == 2, e == 1);
}
test_ret_multi_last_arg();

function test_varargs() {
    closure ret_varargs(...) {
        return ...;
    }

    let a, b, c = ret_varargs(1, 2, 3);
    assert(a == 1, b == 2, c == 3);

    let a, b = ret_varargs(1);
    assert(a == 1, b == undefined);

    let a, b, c, d = ret_varargs(1, 2, ret_varargs(3, 4));
    assert(a == 1, b == 2, c == 3, d == 4);

    closure ret_varargs_pre_ret(...) {
        return 1, 2, ...;
    }

    let a, b, c, d = ret_varargs_pre_ret(3, 4);
    assert(a == 1, b == 2, c == 3, d == 4);

    closure ret_varargs_pre_arg(a, b, ...) {
        return ...;
    }

    let a, b, c = ret_varargs_pre_arg(1, 2, 3, 4, 5);
    assert(a == 3, b == 4, c == 5);

    closure test_let_varargs(ac, bc, cc, ...) {
        let a, b, c = ...;
        assert(a == ac, b == bc, c == cc);
    }
    test_let_varargs(3, 2, 1, 3, 2, 1);

    closure test_varargs_single_position(test, ...) {
        assert(test == ...);
    }
    test_varargs_single_position(4, 4, 3, 2, 1);
}
test_varargs();

return true;

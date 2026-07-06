// Global variables
a = 1;
b = 2;

var test_method = function() {
	return a + b;
};

var obj_a = {
	a: 3,
	b: 4,
};

var obj_b = {
	a: 5,
	b: 6,
};

// By default, functions inherit the `self` of their definition (in this case,
// the globals table).
obj_a.method = test_method;
assert(obj_a.method() == 3);

// We can unbind the default `self` to create a method with no bound `self`.
// When called as a function from a field, it will inherit `self` from the
// object it is the field of.
test_method = method(undefined, test_method);

obj_a.method = test_method;
obj_b.method = test_method;

assert(obj_a.method() == 7);
assert(obj_b.method() == 11);

var test_method_2 = obj_a.method;

// If not called as a field, an unbound function will return to using the `self`
// from the current environment.
assert(test_method_2() == 3);

// As a special case, functions defined inline as the value in a struct
// literal become bound to the instance being constructed.

var obj_c = {
	a: 7,
	b: 8,
	get: function() {
		return a + b;
	},
};

assert(obj_c.get() == 15);
var f = obj_c.get;
assert(f() == 15);

// This ONLY applies if the member value is a function expression *inline* in
// the struct literal.

var obj_d = {
	a: 9,
	b: 10,
	get: undefined,
};

obj_d.get = function() {
	return a + b;
};

assert(obj_d.get() == 3);

return true;

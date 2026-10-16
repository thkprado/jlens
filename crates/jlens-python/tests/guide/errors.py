from jlens import Expression, JlensError

# ANCHOR: errors
try:
    Expression("Account.Order[0;]")
except JlensError as error:
    assert error.code == "S0202"
    assert str(error) == 'Expected "]", got ";"'
    assert error.position == 16  # where it was detected, as in jsonata-js
    assert (error.start, error.end) == (15, 16)  # the offending part of the source
    assert error.token == ";"
# ANCHOR_END: errors
else:
    raise AssertionError("no error")

# ANCHOR: error_value
try:
    Expression("customer.name + 1").evaluate({"customer": {"name": "Acme"}})
except JlensError as error:
    assert error.code == "T2001"
    assert error.value == "Acme"
# ANCHOR_END: error_value
else:
    raise AssertionError("no error")

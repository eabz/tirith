"""Money helpers: rounding, formatting and parsing of ``Decimal`` amounts."""

from decimal import ROUND_HALF_UP, Decimal
from typing import Union

# Minor-unit digits per supported currency.
CURRENCY_DECIMALS = {"USD": 2, "EUR": 2, "JPY": 0}


def to_decimal(value: Union[Decimal, int, str]) -> Decimal:
    """Convert an int, str or Decimal to Decimal; floats are refused."""
    if isinstance(value, float):
        raise TypeError("use Decimal or str for money, not float")
    return value if isinstance(value, Decimal) else Decimal(value)


def round_money(amount: Decimal, currency: str) -> Decimal:
    """Round to the currency's minor unit, half up."""
    if currency not in CURRENCY_DECIMALS:
        raise ValueError("unsupported currency: %r" % currency)
    exponent = Decimal(1).scaleb(-CURRENCY_DECIMALS[currency])
    return to_decimal(amount).quantize(exponent, rounding=ROUND_HALF_UP)


def format_money(amount: Decimal, currency: str) -> str:
    """``format_money(Decimal("3.5"), "USD") == "3.50 USD"``."""
    return "%s %s" % (round_money(amount, currency), currency)


def parse_money(text: str) -> Decimal:
    """Parse ``"12.50"``, ``"$1,234.50"``, ``"1234 JPY"``, ``"(12.00)"``, ``"-3"``."""
    s = text.strip()
    negative = False
    if s.startswith("(") and s.endswith(")"):
        negative, s = True, s[1:-1].strip()
    if len(s) > 4 and s[-4] == " " and s[-3:].isalpha() and s[-3:].isupper():
        s = s[:-4].rstrip()
    if s.startswith("-"):
        if negative:
            raise ValueError("not an amount: %r" % text)
        negative, s = True, s[1:].lstrip()
    if s[:1] in ("$", "\u20ac", "\u00a5"):
        s = s[1:]
    whole, dot, fraction = s.replace(",", "").partition(".")
    if not whole.isdigit() or (dot and not fraction.isdigit()):
        raise ValueError("not an amount: %r" % text)
    value = Decimal(whole + dot + fraction)
    return -value if negative else value

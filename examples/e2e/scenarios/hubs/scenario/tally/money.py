"""Money helpers: rounding, formatting and parsing of ``Decimal`` amounts."""

from decimal import ROUND_HALF_UP, Decimal
from typing import Union

# Minor-unit digits per supported currency.
CURRENCY_DECIMALS = {"USD": 2, "EUR": 2}


def to_decimal(value: Union[Decimal, int, str]) -> Decimal:
    """Convert an int, str or Decimal to Decimal; floats are refused."""
    if isinstance(value, float):
        raise TypeError("use Decimal or str for money, not float")
    return value if isinstance(value, Decimal) else Decimal(value)


def round_money(amount: Decimal) -> Decimal:
    """Round to cents, half up."""
    return to_decimal(amount).quantize(Decimal("0.01"), rounding=ROUND_HALF_UP)


def format_money(amount: Decimal, currency: str) -> str:
    """``format_money(Decimal("3.5"), "USD") == "3.50 USD"``."""
    return "%s %s" % (round_money(amount), currency)


def parse_money(text: str) -> Decimal:
    """Parse a plain decimal amount such as ``"12.50"``."""
    return Decimal(text.strip())

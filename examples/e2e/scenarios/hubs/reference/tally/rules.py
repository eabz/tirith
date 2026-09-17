"""Business tables. Pricing code reads them as ``rules.NAME`` so a new
table needs no import changes elsewhere."""

from decimal import Decimal

# Discount code -> rate of the merchandise subtotal. Codes are matched
# case-insensitively.
DISCOUNT_CODES = {
    "WELCOME10": Decimal("0.10"),
    "VIP20": Decimal("0.20"),
    "SPRING15": Decimal("0.15"),
}

# Region -> sales tax rate on merchandise after discount.
TAX_RATES = {
    "US-CA": Decimal("0.0725"),
    "US-NY": Decimal("0.04"),
    "US-TX": Decimal("0.0625"),
    "EU-DE": Decimal("0.19"),
    "EU-FR": Decimal("0.20"),
    "JP-13": Decimal("0.10"),
}

# Zone -> (base fee, fee per kg), in the zone's usual currency.
SHIPPING_RATES = {
    "US": (Decimal("5.00"), Decimal("1.50")),
    "EU": (Decimal("7.00"), Decimal("2.00")),
    "JP": (Decimal("800"), Decimal("200")),
}


def zone_for(region: str) -> str:
    """``zone_for("US-CA") == "US"``."""
    return region.split("-", 1)[0].upper()


FREE_SHIPPING_OVER = {
    "US": Decimal("100.00"),
    "EU": Decimal("120.00"),
    "JP": Decimal("15000"),
}

TAX_EXEMPT_CATEGORIES = frozenset({"books", "food"})

GIFT_WRAP_FEE = {
    "USD": Decimal("5.00"),
    "EUR": Decimal("5.00"),
    "JPY": Decimal("700"),
}

SMALL_ORDER_FEE = {
    "USD": (Decimal("20.00"), Decimal("2.50")),
    "EUR": (Decimal("20.00"), Decimal("2.50")),
    "JPY": (Decimal("2500"), Decimal("300")),
}

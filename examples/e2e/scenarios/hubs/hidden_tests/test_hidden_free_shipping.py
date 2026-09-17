"""Hidden acceptance: free-shipping."""

import unittest
from decimal import Decimal

from tally import rules
from tally.models import Item, Order
from tally.pricing import order_total, shipping_cost


def order(region="US-NY", weight="0.5", quantity=2, currency="USD", price="10.00", code=None):
    return Order(items=[Item("S-1", Decimal(price), quantity, Decimal(weight))], region=region, currency=currency,
                 discount_code=code)


class FreeShippingTest(unittest.TestCase):
    def test_thresholds_table(self):
        self.assertEqual(rules.FREE_SHIPPING_OVER,
                         {"US": Decimal("100.00"), "EU": Decimal("120.00"), "JP": Decimal("15000")})

    def test_us_threshold_is_inclusive(self):
        self.assertEqual(shipping_cost(order(), Decimal("100.00")), Decimal("0"))
        self.assertEqual(shipping_cost(order(), Decimal("99.99")), Decimal("6.50"))

    def test_eu_threshold(self):
        self.assertEqual(shipping_cost(order(region="EU-DE"), Decimal("119.99")), Decimal("9.00"))
        self.assertEqual(shipping_cost(order(region="EU-DE"), Decimal("120.00")), Decimal("0"))

    def test_jp_threshold(self):
        self.assertEqual(shipping_cost(order(region="JP-13", currency="JPY"), Decimal("15000")), Decimal("0"))

    def test_heavy_orders_never_ship_free(self):
        self.assertEqual(shipping_cost(order(weight="15.5"), Decimal("500.00")), Decimal("51.50"))
        self.assertEqual(shipping_cost(order(weight="15"), Decimal("500.00")), Decimal("0"))

    def test_threshold_applies_after_discount(self):
        totals = order_total(order(price="55.00", weight="0", code="WELCOME10"))
        self.assertEqual((totals.subtotal, totals.discount, totals.shipping),
                         (Decimal("110.00"), Decimal("11.00"), Decimal("5.00")))

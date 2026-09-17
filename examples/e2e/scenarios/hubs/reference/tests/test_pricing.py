import unittest
from decimal import Decimal

from tally.models import Item, Order
from tally.pricing import apply_discount, order_total, shipping_cost, subtotal, tax_for


def order(**kwargs):
    items = kwargs.pop("items", [Item("A-1", Decimal("19.99"), 2, Decimal("0.5"))])
    return Order(items=items, **kwargs)


class PricingTest(unittest.TestCase):
    def test_subtotal(self):
        self.assertEqual(subtotal(order()), Decimal("39.98"))

    def test_discount_code_case_insensitive(self):
        self.assertEqual(apply_discount(order(discount_code="welcome10"), Decimal("39.98")), Decimal("4.00"))

    def test_unknown_code(self):
        self.assertEqual(apply_discount(order(discount_code="NOPE"), Decimal("10")), Decimal("0.00"))

    def test_shipping_by_weight(self):
        self.assertEqual(shipping_cost(order(), Decimal("39.98")), Decimal("6.50"))

    def test_tax(self):
        self.assertEqual(tax_for(order(region="US-NY"), Decimal("100.00")), Decimal("4.00"))

    def test_total(self):
        totals = order_total(order(discount_code="WELCOME10"))
        self.assertEqual(totals.subtotal, Decimal("39.98"))
        self.assertEqual(totals.discount, Decimal("4.00"))
        self.assertEqual(totals.shipping, Decimal("6.50"))
        self.assertEqual(totals.tax, Decimal("2.61"))
        self.assertEqual(totals.total, Decimal("45.09"))

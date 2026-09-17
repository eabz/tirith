"""Hidden acceptance: bulk-discount."""

import unittest
from decimal import Decimal

from tally import rules
from tally.models import Item, Order
from tally.pricing import apply_discount


def units(*quantities, code=None, price="10.00"):
    return Order(items=[Item("S-%d" % i, Decimal(price), q) for i, q in enumerate(quantities)], region="US-NY",
                 discount_code=code)


class BulkDiscountTest(unittest.TestCase):
    def test_ten_units_get_five_percent(self):
        self.assertEqual(apply_discount(units(10), Decimal("100.00")), Decimal("5.00"))

    def test_nine_units_get_nothing(self):
        self.assertEqual(apply_discount(units(9), Decimal("90.00")), Decimal("0"))

    def test_units_counted_across_lines(self):
        self.assertEqual(apply_discount(units(4, 6), Decimal("100.00")), Decimal("5.00"))

    def test_bulk_adds_to_code(self):
        self.assertEqual(apply_discount(units(10, code="WELCOME10"), Decimal("100.00")), Decimal("15.00"))

    def test_combined_rate_capped(self):
        self.assertEqual(apply_discount(units(12, code="VIP20", price="3.33"), Decimal("39.96")), Decimal("9.99"))

    def test_spring15(self):
        self.assertEqual(rules.DISCOUNT_CODES["SPRING15"], Decimal("0.15"))
        self.assertEqual(apply_discount(units(2, code="spring15"), Decimal("20.00")), Decimal("3.00"))
        self.assertEqual(apply_discount(units(10, code="SPRING15"), Decimal("100.00")), Decimal("20.00"))

    def test_rounds_the_discount(self):
        self.assertEqual(apply_discount(units(10), Decimal("33.33")), Decimal("1.67"))

"""Hidden acceptance: small-order-fee."""

import unittest
from decimal import Decimal

from tally import rules
from tally.models import Item, Order, Totals
from tally.pricing import order_total


class SmallOrderFeeTest(unittest.TestCase):
    def test_field_and_table(self):
        self.assertEqual(Totals(Decimal(1), Decimal(0), Decimal(0), Decimal(0), Decimal(1)).small_order_fee,
                         Decimal("0"))
        self.assertEqual(rules.SMALL_ORDER_FEE["USD"], (Decimal("20.00"), Decimal("2.50")))
        self.assertEqual(rules.SMALL_ORDER_FEE["EUR"], (Decimal("20.00"), Decimal("2.50")))
        self.assertEqual(rules.SMALL_ORDER_FEE["JPY"], (Decimal("2500"), Decimal("300")))

    def test_below_threshold_pays_fee(self):
        totals = order_total(Order(items=[Item("S-1", Decimal("19.99"))], region="US-NY"))
        self.assertEqual((totals.tax, totals.small_order_fee, totals.total),
                         (Decimal("0.80"), Decimal("2.50"), Decimal("28.29")))

    def test_threshold_itself_pays_nothing(self):
        totals = order_total(Order(items=[Item("S-1", Decimal("20.00"))], region="US-NY"))
        self.assertEqual((totals.small_order_fee, totals.total), (Decimal("0"), Decimal("25.80")))

    def test_measured_after_discount(self):
        totals = order_total(Order(items=[Item("S-1", Decimal("21.00"))], region="US-NY", discount_code="WELCOME10"))
        self.assertEqual((totals.discount, totals.small_order_fee), (Decimal("2.10"), Decimal("2.50")))

    def test_empty_order_pays_nothing(self):
        self.assertEqual(order_total(Order(items=[], region="US-NY")).small_order_fee, Decimal("0"))

    def test_jpy(self):
        totals = order_total(Order(items=[Item("S-1", Decimal("2000"))], region="JP-13", currency="JPY"))
        self.assertEqual((totals.small_order_fee, totals.total), (Decimal("300"), Decimal("3300")))

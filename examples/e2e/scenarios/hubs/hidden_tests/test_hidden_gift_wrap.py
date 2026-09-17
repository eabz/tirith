"""Hidden acceptance: gift-wrap."""

import unittest
from decimal import Decimal

from tally import rules
from tally.models import Item, Order, Totals
from tally.pricing import order_total


class GiftWrapTest(unittest.TestCase):
    def test_fields_and_table(self):
        self.assertFalse(Order().gift_wrap)
        self.assertEqual(Totals(Decimal(1), Decimal(0), Decimal(0), Decimal(0), Decimal(1)).gift_wrap_fee, Decimal("0"))
        self.assertEqual(rules.GIFT_WRAP_FEE, {"USD": Decimal("5.00"), "EUR": Decimal("5.00"), "JPY": Decimal("700")})

    def test_fee_added_to_total(self):
        totals = order_total(Order(items=[Item("G-1", Decimal("25.00"), 2)], region="US-NY", gift_wrap=True))
        self.assertEqual((totals.shipping, totals.tax, totals.gift_wrap_fee, totals.total),
                         (Decimal("5.00"), Decimal("2.00"), Decimal("5.00"), Decimal("62.00")))

    def test_fee_not_discounted_or_taxed(self):
        totals = order_total(Order(items=[Item("G-1", Decimal("25.00"), 2)], region="US-NY", gift_wrap=True,
                                   discount_code="WELCOME10"))
        self.assertEqual((totals.discount, totals.tax, totals.total),
                         (Decimal("5.00"), Decimal("1.80"), Decimal("56.80")))

    def test_jpy_fee(self):
        totals = order_total(Order(items=[Item("G-1", Decimal("3000"))], region="JP-13", currency="JPY",
                                   gift_wrap=True))
        self.assertEqual((totals.gift_wrap_fee, totals.total), (Decimal("700"), Decimal("4800")))

    def test_no_fee_without_gift_wrap(self):
        totals = order_total(Order(items=[Item("G-1", Decimal("25.00"), 2)], region="US-NY"))
        self.assertEqual((totals.gift_wrap_fee, totals.total), (Decimal("0"), Decimal("57.00")))

    def test_fee_does_not_count_toward_free_shipping(self):
        totals = order_total(Order(items=[Item("G-1", Decimal("96.00"))], region="US-NY", gift_wrap=True))
        self.assertEqual(totals.shipping, Decimal("5.00"))

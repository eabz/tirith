"""Hidden acceptance: tax-exempt."""

import unittest
from decimal import Decimal

from tally import rules
from tally.models import Item, Order
from tally.pricing import tax_for


class TaxExemptTest(unittest.TestCase):
    def test_exempt_categories(self):
        self.assertEqual(rules.TAX_EXEMPT_CATEGORIES, frozenset({"books", "food"}))

    def test_all_exempt(self):
        order = Order(items=[Item("B-1", Decimal("30.00"), 1, category="books"),
                             Item("F-1", Decimal("20.00"), 1, category="food")], region="US-CA")
        self.assertEqual(tax_for(order, Decimal("50.00")), Decimal("0"))

    def test_discount_is_prorated(self):
        order = Order(items=[Item("G-1", Decimal("70.00"), 1), Item("F-1", Decimal("30.00"), 1, category="food")],
                      region="US-CA", discount_code="WELCOME10")
        self.assertEqual(tax_for(order, Decimal("90.00")), Decimal("4.57"))

    def test_new_rates(self):
        self.assertEqual(rules.TAX_RATES["US-TX"], Decimal("0.0625"))
        self.assertEqual(rules.TAX_RATES["EU-FR"], Decimal("0.20"))
        texas = Order(items=[Item("G-1", Decimal("100.00"))], region="US-TX")
        self.assertEqual(tax_for(texas, Decimal("100.00")), Decimal("6.25"))
        france = Order(items=[Item("G-1", Decimal("50.00"))], region="EU-FR", currency="EUR")
        self.assertEqual(tax_for(france, Decimal("50.00")), Decimal("10.00"))

    def test_general_items_still_taxed(self):
        order = Order(items=[Item("G-1", Decimal("100.00"))], region="US-NY")
        self.assertEqual(tax_for(order, Decimal("100.00")), Decimal("4.00"))

    def test_empty_order(self):
        self.assertEqual(tax_for(Order(items=[], region="US-CA"), Decimal("0")), Decimal("0"))

"""This plugin's models, by model id: replace them with your own.

A model is a function of one pricing unit -- one instrument under one
scenario, with exactly the market objects its model declares -- answering
its value per unit as a `meridian.Money`. Here `quoted_price` is the
built-in model's own computation (`meridian.roles.pricing.reference_model`):
the instrument's price times each conversion leg the unit carries, exact,
so its answers equal the built-in implementation's on every input.

A model reads nothing but the batch and the unit: no position, no account,
no other price. A model raising for a unit leaves the batch unanswered.
"""

from __future__ import annotations

from meridian.roles import pricing

#: The model ids this plugin implements, and how.
MODELS: dict[str, pricing.Model] = {"quoted_price": pricing.reference_model}

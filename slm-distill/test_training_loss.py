"""Chunking must preserve answer-only loss and gradients across masked prompts."""
from types import SimpleNamespace

import pytest
import torch
from torch.nn import functional as F

from train import AnswerOnlyTrainer


@pytest.mark.parametrize('chunk_size', [64, 256])
@pytest.mark.parametrize('head_dtype', [torch.float32, torch.bfloat16])
@pytest.mark.parametrize('batch_token_count', [None, 427])
def test_checkpointed_loss_matches_full_vocabulary_loss_and_gradients(batch_token_count, head_dtype, chunk_size):
    torch.manual_seed(13)
    head = torch.nn.Linear(32, 1024, bias=False).to(head_dtype).requires_grad_(False)
    labels = torch.randint(0, 1024, (1, 320))
    labels[:, :37] = -100
    full_hidden = torch.randn(1, 320, 32, requires_grad=True)
    chunked_hidden = full_hidden.detach().clone().requires_grad_()
    keep = labels[:, 1:] != -100
    denominator = batch_token_count if batch_token_count is not None else keep.sum()
    expected = F.cross_entropy(head(full_hidden[:, :-1][keep].to(head_dtype)).float(), labels[:, 1:][keep], reduction='sum') / denominator
    expected.backward()
    inner = SimpleNamespace(lm_head=head, model=lambda **_: SimpleNamespace(last_hidden_state=chunked_hidden))
    model = SimpleNamespace(base_model=SimpleNamespace(model=inner))
    trainer = object.__new__(AnswerOnlyTrainer)
    trainer.loss_chunk_size = chunk_size
    actual = trainer.compute_loss(model, {'labels': labels, 'input_ids': labels, 'attention_mask': torch.ones_like(labels)},
                                  num_items_in_batch=batch_token_count)
    actual.backward()
    torch.testing.assert_close(actual, expected)
    torch.testing.assert_close(chunked_hidden.grad, full_hidden.grad)

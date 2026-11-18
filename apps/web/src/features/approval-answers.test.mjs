import { test } from "node:test";
import assert from "node:assert/strict";
import { OTHER_ANSWER, useApprovalAnswers } from "./approval-answers.ts";

const input = (value, checked) => ({ target: { value, checked } });
const item = {
  type: "approval",
  id: "a1",
  title: "Two details",
  text: "",
  choices: ["accept", "cancel"],
  resolved: false,
  questions: [
    { id: "layout", question: "Where?", options: [{ label: "Thread" }, { label: "Composer" }], isOther: true },
    { id: "targets", question: "Which?", multiSelect: true, options: [{ label: "A" }, { label: "B" }] },
  ],
};

test("answers are collected per question until every question has one", () => {
  const answers = useApprovalAnswers();
  assert.equal(answers.incomplete(item), true);
  answers.answer("a1", "layout", input("Thread"));
  assert.equal(answers.incomplete(item), true, "one question is still open");
  answers.toggle("a1", "targets", "A", input("A", true));
  answers.toggle("a1", "targets", "B", input("B", true));
  answers.toggle("a1", "targets", "A", input("A", false));
  assert.equal(answers.incomplete(item), false);
  assert.deepEqual(answers.payload(item), { layout: ["Thread"], targets: ["B"] });
});

test("a typed other answer replaces a single choice, and choosing an option again drops it", () => {
  const answers = useApprovalAnswers();
  answers.answer("a1", "layout", input("Thread"));
  answers.typeOther("a1", "layout", input("Beside the diff"));
  assert.deepEqual(answers.payload(item).layout, ["Beside the diff"]);
  answers.answer("a1", "layout", input(OTHER_ANSWER));
  assert.deepEqual(answers.chosen.a1.layout, [], "the other radio itself is no answer");
  answers.answer("a1", "layout", input("Composer"));
  assert.equal(answers.other.a1.layout, undefined);
  assert.deepEqual(answers.payload(item).layout, ["Composer"]);
});

test("a resolved request forgets its answers; a reset forgets them all", () => {
  const answers = useApprovalAnswers();
  answers.answer("a1", "layout", input("Thread"));
  answers.answer("a2", "x", input("y"));
  answers.clear("a1");
  assert.equal(answers.chosen.a1, undefined);
  assert.deepEqual({ ...answers.chosen.a2 }, { x: ["y"] });
  answers.clear();
  assert.deepEqual(Object.keys(answers.chosen), []);
});

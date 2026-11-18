// A plain JavaScript module.

/** Makes a store. */
function createStore(reducer, initial) {
  let state = initial;
  const listeners = [];
  return {
    getState() { return state; },
  };
}

export const VERSION = "1.0";

class Store {
  constructor(reducer) {
    this.reducer = reducer;
  }

  dispatch(action) {
    if (action) {
      this.state = this.reducer(this.state, action);
    }
  }

  static of(reducer) {
    return new Store(reducer);
  }
}

const config = {
  name: "x",
  run() {},
};

exports.helper = (x) => x * 2;

describe("store", () => {
  it("works", () => {});
});

export function trim(p) {
  return p.replace(/\/*$/, "");
}

export function afterTrim() {}

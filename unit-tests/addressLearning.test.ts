import { describe, expect, test } from "bun:test";
import {
  addressChanges,
  findAddresses,
  normaliseAddress,
} from "../src/lib/addressLearning";

// Invented addresses only.
describe("addressLearning", () => {
  test("finds addresses, lower case, without a trailing full stop", () => {
    expect(
      findAddresses("Email Nick@Kite.co.nz. Or sam.smith@gmail.com, thanks"),
    ).toEqual(["nick@kite.co.nz", "sam.smith@gmail.com"]);
    expect(findAddresses("no address here, kyt.nz is a site")).toEqual([]);
  });

  test("normalises user-entered addresses", () => {
    expect(normaliseAddress("  Nic@Kyt.NZ ")).toBe("nic@kyt.nz");
    expect(normaliseAddress("<nic@kyt.nz>")).toBe("nic@kyt.nz");
    expect(normaliseAddress("kyt.nz")).toBeNull();
    expect(normaliseAddress("nic@")).toBeNull();
    expect(normaliseAddress("a b@c.nz")).toBeNull();
  });

  test("an address replaced by another is learnt", () => {
    expect(
      addressChanges(
        "Send it to nick@kite.co.nz please",
        "Send it to nic@kyt.nz please",
      ),
    ).toEqual([{ heard: "nick@kite.co.nz", means: "nic@kyt.nz" }]);
  });

  test("edits that do not swap an address learn nothing", () => {
    expect(addressChanges("costs a lot", "cost a lot")).toEqual([]);
    expect(
      addressChanges("mail nic@kyt.nz", "Please mail nic@kyt.nz today"),
    ).toEqual([]);
    expect(addressChanges("mail nic@kyt.nz", "mail")).toEqual([]);
    expect(addressChanges("mail", "mail nic@kyt.nz")).toEqual([]);
    expect(addressChanges("Mail Nic@Kyt.nz", "mail nic@kyt.nz")).toEqual([]);
  });

  test("an exact correction already taught is not offered again", () => {
    expect(
      addressChanges("to nick@kite.co.nz", "to nic@kyt.nz", [
        { wrong: "nick@kite.co.nz", right: "nic@kyt.nz" },
      ]),
    ).toEqual([]);
  });

  test("two swaps pair in order", () => {
    expect(addressChanges("a@b.nz and c@d.nz", "x@y.nz and z@w.nz")).toEqual([
      { heard: "a@b.nz", means: "x@y.nz" },
      { heard: "c@d.nz", means: "z@w.nz" },
    ]);
  });

  test("the user's own address corrected to someone else's is never taught", () => {
    expect(
      addressChanges("to nic@kyt.nz", "to nick@kyt.nz", [], ["Nic@kyt.nz"]),
    ).toEqual([]);
  });
});

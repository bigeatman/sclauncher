using System;
using System.Drawing;
using ScMultiTest;

internal static class SelectionHudTests
{
    private static int passed;
    private static void Check(bool condition, string name)
    {
        if (!condition) throw new Exception("FAILED: selection HUD: " + name);
        passed++;
    }
    private static void Reject(string text, string name)
    {
        try { Snapshot.Parse(text, 701); }
        catch (FormatException) { passed++; return; }
        throw new Exception("FAILED: selection HUD invalid snapshot accepted: " + name);
    }
    private static Snapshot Parse(string suffix)
    {
        return Snapshot.Parse("SCMULTI2\t701\t0\t0\t37\t9\tREADY\ttest\t" + suffix, 701);
    }
    internal static int Run()
    {
        passed = 0;
        Snapshot empty = Parse("1\t0\t65535"), selected = Parse("1\t1\t41"), mixed = Parse("1\t5\t65535");
        Snapshot active = Snapshot.Parse("SCMULTI2\t701\t1\t15\t37\t9\tREADY\ttest\t1\t15\t37", 701);
        Check(empty.ProtocolVersion == 2 && empty.InGame && !empty.Active && empty.DisplayCount == 0, "empty in-game selection");
        Check(SelectionHud.ShouldShow(empty) && SelectionHud.NameText(empty) == "<없음>" && SelectionHud.CountText(empty) == "0", "empty HUD stays visible");
        Check(selected.Count == 0 && selected.DisplayCount == 1 && selected.DisplayUnitType == 41 && selected.UnitType == 37, "ordinary selection independent of prior virtual kind");
        Check(SelectionHud.NameText(selected) == "드론" && SelectionHud.CountText(selected) == "1", "single drone display");
        Check(SelectionHud.NameText(mixed) == "여러 종류" && SelectionHud.CountText(mixed) == "5", "mixed ordinary selection display");
        Check(active.Active && active.Count == 15 && active.DisplayCount == 15 && SelectionHud.NameText(active) == "저글링", "active group display");
        Check(!SelectionHud.ShouldShow(Parse("0\t0\t65535")), "menu hidden");
        Check(!SelectionHud.ShouldShow(null), "missing status hidden");
        Check(!SelectionHud.ShouldShow(Snapshot.Parse("SCMULTI2\t701\t0\t0\t0\t0\tFAILED\terror\t0\t0\t65535", 701)), "failed connection hidden");
        Check(!SelectionHud.ShouldShow(Snapshot.Parse("SCMULTI2\t701\t0\t0\t0\t0\tWAITING\tmenu\t0\t0\t65535", 701)), "waiting connection hidden");
        Check(!SelectionHud.ShouldShow(Snapshot.Parse("SCMULTI1\t701\t0\t0\t0\t0\tREADY\tready", 701)), "legacy inactive cannot imply in-game");
        Snapshot legacy = Snapshot.Parse("SCMULTI1\t701\t1\t15\t37\t24\tREADY\tactive", 701);
        Check(legacy.ProtocolVersion == 1 && legacy.InGame && legacy.DisplayCount == 15 && SelectionHud.ShouldShow(legacy), "legacy active compatibility");
        Check(SelectionHud.CountText(Snapshot.Parse("SCMULTI2\t701\t1\t8192\t64\t0\tREADY\tmax\t1\t8192\t64", 701)) == "8192", "maximum virtual group");
        Reject("SCMULTI2\t701\t0\t0\t0\t0\tREADY\tx\t2\t0\t65535", "invalid game flag");
        Reject("SCMULTI2\t701\t0\t0\t0\t0\tREADY\tx\t0\t1\t41", "selection outside game");
        Reject("SCMULTI2\t701\t0\t0\t0\t0\tREADY\tx\t1\t0\t41", "empty selection stale kind");
        Reject("SCMULTI2\t701\t0\t0\t0\t0\tREADY\tx\t1\t8193\t41", "display count limit");
        Reject("SCMULTI2\t701\t0\t0\t0\t0\tREADY\tx\t1\t1\t65536", "display kind limit");
        Reject("SCMULTI2\t701\t0\t0\t0\t0\tREADY\tx\t1\t+1\t41", "signed count");
        Reject("SCMULTI2\t701\t1\t15\t37\t0\tREADY\tx\t0\t0\t65535", "active outside game");
        Reject("SCMULTI2\t701\t1\t15\t37\t0\tREADY\tx\t1\t14\t37", "active group count mismatch");
        Reject("SCMULTI2\t701\t1\t15\t37\t0\tREADY\tx\t1\t15\t41", "active group kind mismatch");
        Reject("SCMULTI2\t701\t1\t15\t65535\t0\tREADY\tx\t1\t15\t65535", "mixed virtual group");
        Reject("SCMULTI2\t701\t0\t0\t0\t0\tREADY\tx\t1\t0", "missing display kind");
        Reject("SCMULTI2\t701\t0\t0\t0\t0\tREADY\tx\t1\t0\t65535\textra", "extra display field");
        Check(UnitNames.Get(0) == "마린" && UnitNames.Get(7) == "SCV" && UnitNames.Get(35) == "라바", "terran and larva IDs");
        Check(UnitNames.Get(37) == "저글링" && UnitNames.Get(41) == "드론" && UnitNames.Get(64) == "프로브", "standard worker IDs");
        Check(UnitNames.Get(103) == "럴커" && UnitNames.Get(106) == "커맨드 센터" && UnitNames.Get(160) == "게이트웨이", "expansion and building IDs");
        Check(UnitNames.Get(119) == "유닛 #119" && UnitNames.Get(4000) == "유닛 #4000", "unmapped ID explicit fallback");
        Rectangle region;
        Check(SelectionHud.TryBounds(new Rectangle(0, 0, 1920, 1200), out region) && region == new Rectangle(360, 995, 155, 126), "reference recess");
        Check(SelectionHud.TryBounds(new Rectangle(40, 80, 1920, 1200), out region) && region == new Rectangle(400, 1075, 155, 126), "window client origin");
        Check(SelectionHud.TryBounds(new Rectangle(0, 0, 1280, 960), out region) && region.Left >= 272 && region.Right <= 352, "4:3 minimap portrait gap");
        Check(SelectionHud.TryBounds(new Rectangle(0, 0, 640, 480), out region) && region.Left == 144 && region.Right == 174, "small 4:3 gap");
        Check(SelectionHud.TryBounds(new Rectangle(0, 0, 1920, 1080), out region) && region.Left == 324 && region.Right <= 465, "16:9 gap");
        Check(!SelectionHud.TryBounds(Rectangle.Empty, out region), "missing game bounds");
        Check(!SelectionHud.TryBounds(new Rectangle(0, 0, 200, 150), out region), "unsupported small window");
        Check(!SelectionHud.TryBounds(new Rectangle(0, 0, 320, 800), out region), "unsupported portrait window");
        using (var bitmap = new Bitmap(240, 180))
        using (Graphics graphics = Graphics.FromImage(bitmap))
        {
            graphics.Clear(Color.Magenta);
            var local = new Rectangle(40, 30, 155, 126);
            SelectionHud.Draw(graphics, active, local);
            bool ink = false, outside = false;
            int background = Color.Magenta.ToArgb();
            for (int y = 0; y < bitmap.Height; y++)
                for (int x = 0; x < bitmap.Width; x++)
                    if (bitmap.GetPixel(x, y).ToArgb() != background)
                    { if (local.Contains(x, y)) ink = true; else outside = true; }
            Check(ink && !outside, "HUD draws only inside carved region");
            graphics.Clear(Color.Magenta);
            SelectionHud.Draw(graphics, Parse("0\t0\t65535"), local);
            Check(bitmap.GetPixel(100, 100).ToArgb() == background, "menu draws nothing");
            SelectionHud.Draw(graphics, selected, new Rectangle(0, 0, 30, 50));
            Check(true, "small window font fits without failure");
        }
        return passed;
    }
}
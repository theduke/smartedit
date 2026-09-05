namespace Fixtures.Legacy
{
    public struct Counter
    {
        public int Value;

        public void Increment()
        {
            Value++;
        }

        public int Add(int amount)
        {
            Value += amount;
            return Value;
        }
    }
}
